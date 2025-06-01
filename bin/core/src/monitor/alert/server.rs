use std::{
  collections::HashMap,
  path::PathBuf,
  str::FromStr,
  sync::{Mutex, OnceLock},
};

use anyhow::Context;
use derive_variants::ExtractVariant;
use komodo_client::entities::{
  ResourceTarget,
  alert::{Alert, AlertData, AlertDataVariant, SeverityLevel},
  komodo_timestamp, optional_string,
  server::{Server, ServerState},
};
use mongo_indexed::Indexed;
use mungos::{
  bulk_update::{self, BulkUpdate},
  find::find_collect,
  mongodb::bson::{doc, oid::ObjectId, to_bson},
};

use crate::{
  alert::send_alerts,
  state::{db_client, server_status_cache},
};

type SendAlerts = bool;
type OpenAlertMap<T = AlertDataVariant> =
  HashMap<ResourceTarget, HashMap<T, Alert>>;
type OpenDiskAlertMap = OpenAlertMap<PathBuf>;

#[instrument(level = "debug")]
pub async fn alert_servers(
  ts: i64,
  mut servers: HashMap<String, Server>,
) {
  let server_statuses = server_status_cache().get_list().await;

  let (open_alerts, open_disk_alerts) = match get_open_alerts().await
  {
    Ok(alerts) => alerts,
    Err(e) => {
      error!("{e:#}");
      return;
    }
  };

  let mut alerts_to_open = Vec::<(Alert, SendAlerts)>::new();
  let mut alerts_to_update = Vec::<(Alert, SendAlerts)>::new();
  let mut alert_ids_to_close = Vec::<(Alert, SendAlerts)>::new();

  let buffer = alert_buffer();

  for server_status in server_statuses {
    let Some(server) = servers.remove(&server_status.id) else {
      continue;
    };
    let server_alerts = open_alerts
      .get(&ResourceTarget::Server(server_status.id.clone()));

    // ===================
    // SERVER HEALTH
    // ===================
    let health_alert = server_alerts.as_ref().and_then(|alerts| {
      alerts.get(&AlertDataVariant::ServerUnreachable)
    });
    match (server_status.state, health_alert) {
      (ServerState::NotOk, None) => {
        if buffer.ready_to_open(
          server_status.id.clone(),
          AlertDataVariant::ServerUnreachable,
        ) {
          // open unreachable alert
          let alert = Alert {
            id: Default::default(),
            ts,
            resolved: false,
            resolved_ts: None,
            level: SeverityLevel::Critical,
            target: ResourceTarget::Server(server_status.id.clone()),
            data: AlertData::ServerUnreachable {
              id: server_status.id.clone(),
              name: server.name.clone(),
              region: optional_string(&server.config.region),
              err: server_status.err.clone(),
            },
          };
          alerts_to_open
            .push((alert, server.config.send_unreachable_alerts))
        }
      }
      (ServerState::NotOk, Some(alert)) => {
        // update alert err
        let mut alert = alert.clone();
        let (id, name, region) = match alert.data {
          AlertData::ServerUnreachable {
            id, name, region, ..
          } => (id, name, region),
          data => {
            error!(
              "got incorrect alert data in ServerStatus handler. got {data:?}"
            );
            continue;
          }
        };
        alert.data = AlertData::ServerUnreachable {
          id,
          name,
          region,
          err: server_status.err.clone(),
        };

        // Never send this alert, severity is always 'Critical'
        alerts_to_update.push((alert, false));
      }

      // Close an open alert
      (ServerState::Ok | ServerState::Disabled, Some(alert)) => {
        alert_ids_to_close.push((
          alert.clone(),
          server.config.send_unreachable_alerts,
        ));
      }
      (ServerState::Ok | ServerState::Disabled, None) => buffer
        .reset(
          server_status.id.clone(),
          AlertDataVariant::ServerUnreachable,
        ),
    }

    let Some(health) = &server_status.health else {
      continue;
    };

    // ===================
    // SERVER CPU
    // ===================
    let cpu_alert = server_alerts
      .as_ref()
      .and_then(|alerts| alerts.get(&AlertDataVariant::ServerCpu))
      .cloned();
    match (health.cpu.level, cpu_alert, health.cpu.should_close_alert)
    {
      (SeverityLevel::Warning | SeverityLevel::Critical, None, _) => {
        // open alert
        if buffer.ready_to_open(
          server_status.id.clone(),
          AlertDataVariant::ServerCpu,
        ) {
          let alert = Alert {
            id: Default::default(),
            ts,
            resolved: false,
            resolved_ts: None,
            level: health.cpu.level,
            target: ResourceTarget::Server(server_status.id.clone()),
            data: AlertData::ServerCpu {
              id: server_status.id.clone(),
              name: server.name.clone(),
              region: optional_string(&server.config.region),
              percentage: server_status
                .stats
                .as_ref()
                .map(|s| s.cpu_perc as f64)
                .unwrap_or(0.0),
            },
          };
          alerts_to_open.push((alert, server.config.send_cpu_alerts));
        }
      }
      (
        SeverityLevel::Warning | SeverityLevel::Critical,
        Some(mut alert),
        _,
      ) => {
        // modify alert level only if it has increased
        if alert.level < health.cpu.level {
          alert.level = health.cpu.level;
          alert.data = AlertData::ServerCpu {
            id: server_status.id.clone(),
            name: server.name.clone(),
            region: optional_string(&server.config.region),
            percentage: server_status
              .stats
              .as_ref()
              .map(|s| s.cpu_perc as f64)
              .unwrap_or(0.0),
          };
          alerts_to_update
            .push((alert, server.config.send_cpu_alerts));
        }
      }
      (SeverityLevel::Ok, Some(alert), true) => {
        let mut alert = alert.clone();
        alert.data = AlertData::ServerCpu {
          id: server_status.id.clone(),
          name: server.name.clone(),
          region: optional_string(&server.config.region),
          percentage: server_status
            .stats
            .as_ref()
            .map(|s| s.cpu_perc as f64)
            .unwrap_or(0.0),
        };
        alert_ids_to_close
          .push((alert, server.config.send_cpu_alerts))
      }
      (SeverityLevel::Ok, Some(_), false) => {}
      (SeverityLevel::Ok, None, _) => buffer
        .reset(server_status.id.clone(), AlertDataVariant::ServerCpu),
    }

    // ===================
    // SERVER MEM
    // ===================
    let mem_alert = server_alerts
      .as_ref()
      .and_then(|alerts| alerts.get(&AlertDataVariant::ServerMem))
      .cloned();
    match (health.mem.level, mem_alert, health.mem.should_close_alert)
    {
      (SeverityLevel::Warning | SeverityLevel::Critical, None, _) => {
        // open alert
        if buffer.ready_to_open(
          server_status.id.clone(),
          AlertDataVariant::ServerMem,
        ) {
          let alert = Alert {
            id: Default::default(),
            ts,
            resolved: false,
            resolved_ts: None,
            level: health.mem.level,
            target: ResourceTarget::Server(server_status.id.clone()),
            data: AlertData::ServerMem {
              id: server_status.id.clone(),
              name: server.name.clone(),
              region: optional_string(&server.config.region),
              total_gb: server_status
                .stats
                .as_ref()
                .map(|s| s.mem_total_gb)
                .unwrap_or(0.0),
              used_gb: server_status
                .stats
                .as_ref()
                .map(|s| s.mem_used_gb)
                .unwrap_or(0.0),
            },
          };
          alerts_to_open.push((alert, server.config.send_mem_alerts));
        }
      }
      (
        SeverityLevel::Warning | SeverityLevel::Critical,
        Some(mut alert),
        _,
      ) => {
        // modify alert level only if it has increased
        if alert.level < health.mem.level {
          alert.level = health.mem.level;
          alert.data = AlertData::ServerMem {
            id: server_status.id.clone(),
            name: server.name.clone(),
            region: optional_string(&server.config.region),
            total_gb: server_status
              .stats
              .as_ref()
              .map(|s| s.mem_total_gb)
              .unwrap_or(0.0),
            used_gb: server_status
              .stats
              .as_ref()
              .map(|s| s.mem_used_gb)
              .unwrap_or(0.0),
          };
          alerts_to_update
            .push((alert, server.config.send_mem_alerts));
        }
      }
      (SeverityLevel::Ok, Some(alert), true) => {
        let mut alert = alert.clone();
        alert.data = AlertData::ServerMem {
          id: server_status.id.clone(),
          name: server.name.clone(),
          region: optional_string(&server.config.region),
          total_gb: server_status
            .stats
            .as_ref()
            .map(|s| s.mem_total_gb)
            .unwrap_or(0.0),
          used_gb: server_status
            .stats
            .as_ref()
            .map(|s| s.mem_used_gb)
            .unwrap_or(0.0),
        };
        alert_ids_to_close
          .push((alert, server.config.send_mem_alerts))
      }
      (SeverityLevel::Ok, Some(_), false) => {}
      (SeverityLevel::Ok, None, _) => buffer
        .reset(server_status.id.clone(), AlertDataVariant::ServerMem),
    }

    // ===================
    // SERVER DISK
    // ===================

    let server_disk_alerts = open_disk_alerts
      .get(&ResourceTarget::Server(server_status.id.clone()));

    for (path, health) in &health.disks {
      let disk_alert = server_disk_alerts
        .as_ref()
        .and_then(|alerts| alerts.get(path))
        .cloned();
      match (health.level, disk_alert, health.should_close_alert) {
        (
          SeverityLevel::Warning | SeverityLevel::Critical,
          None,
          _,
        ) => {
          // open alert
          if buffer.ready_to_open(
            server_status.id.clone(),
            AlertDataVariant::ServerDisk,
          ) {
            let disk =
              server_status.stats.as_ref().and_then(|stats| {
                stats.disks.iter().find(|disk| disk.mount == *path)
              });
            let alert = Alert {
              id: Default::default(),
              ts,
              resolved: false,
              resolved_ts: None,
              level: health.level,
              target: ResourceTarget::Server(
                server_status.id.clone(),
              ),
              data: AlertData::ServerDisk {
                id: server_status.id.clone(),
                name: server.name.clone(),
                region: optional_string(&server.config.region),
                path: path.to_owned(),
                total_gb: disk
                  .map(|d| d.total_gb)
                  .unwrap_or_default(),
                used_gb: disk.map(|d| d.used_gb).unwrap_or_default(),
              },
            };
            alerts_to_open
              .push((alert, server.config.send_disk_alerts));
          }
        }
        (
          SeverityLevel::Warning | SeverityLevel::Critical,
          Some(mut alert),
          _,
        ) => {
          // modify alert level only if it has increased
          if health.level < alert.level {
            let disk =
              server_status.stats.as_ref().and_then(|stats| {
                stats.disks.iter().find(|disk| disk.mount == *path)
              });
            alert.level = health.level;
            alert.data = AlertData::ServerDisk {
              id: server_status.id.clone(),
              name: server.name.clone(),
              region: optional_string(&server.config.region),
              path: path.to_owned(),
              total_gb: disk.map(|d| d.total_gb).unwrap_or_default(),
              used_gb: disk.map(|d| d.used_gb).unwrap_or_default(),
            };
            alerts_to_update
              .push((alert, server.config.send_disk_alerts));
          }
        }
        (SeverityLevel::Ok, Some(alert), true) => {
          let mut alert = alert.clone();
          let disk = server_status.stats.as_ref().and_then(|stats| {
            stats.disks.iter().find(|disk| disk.mount == *path)
          });
          alert.level = health.level;
          alert.data = AlertData::ServerDisk {
            id: server_status.id.clone(),
            name: server.name.clone(),
            region: optional_string(&server.config.region),
            path: path.to_owned(),
            total_gb: disk.map(|d| d.total_gb).unwrap_or_default(),
            used_gb: disk.map(|d| d.used_gb).unwrap_or_default(),
          };
          alert_ids_to_close
            .push((alert, server.config.send_disk_alerts))
        }
        (SeverityLevel::Ok, Some(_), false) => {}
        (SeverityLevel::Ok, None, _) => buffer.reset(
          server_status.id.clone(),
          AlertDataVariant::ServerDisk,
        ),
      }
    }

    // Need to close any open ones on disks no longer reported
    if let Some(disk_alerts) = server_disk_alerts {
      for (path, alert) in disk_alerts {
        if !health.disks.contains_key(path) {
          let mut alert = alert.clone();
          alert.level = SeverityLevel::Ok;
          alert_ids_to_close
            .push((alert, server.config.send_disk_alerts));
        }
      }
    }
  }

  tokio::join!(
    open_new_alerts(&alerts_to_open),
    update_alerts(&alerts_to_update),
    resolve_alerts(&alert_ids_to_close),
  );
}

#[instrument(level = "debug")]
async fn open_new_alerts(alerts: &[(Alert, SendAlerts)]) {
  if alerts.is_empty() {
    return;
  }

  let db = db_client();

  let open = || async {
    let ids = db
      .alerts
      .insert_many(alerts.iter().map(|(alert, _)| alert))
      .await?
      .inserted_ids
      .into_iter()
      .filter_map(|(index, id)| {
        alerts.get(index)?.1.then(|| id.as_object_id())
      })
      .flatten()
      .collect::<Vec<_>>();
    anyhow::Ok(ids)
  };

  let ids_to_send = match open().await {
    Ok(ids) => ids,
    Err(e) => {
      error!("failed to open alerts on db | {e:?}");
      return;
    }
  };

  let alerts = match find_collect(
    &db.alerts,
    doc! { "_id": { "$in": ids_to_send } },
    None,
  )
  .await
  {
    Ok(alerts) => alerts,
    Err(e) => {
      error!("failed to pull created alerts from mongo | {e:?}");
      return;
    }
  };

  send_alerts(&alerts).await
}

#[instrument(level = "debug")]
async fn update_alerts(alerts: &[(Alert, SendAlerts)]) {
  if alerts.is_empty() {
    return;
  }

  let open = || async {
    let updates = alerts.iter().map(|(alert, _)| {
        let update = BulkUpdate {
          query: doc! { "_id": ObjectId::from_str(&alert.id).context("failed to convert alert id to ObjectId")? },
          update: doc! { "$set": to_bson(alert).context("failed to convert alert to bson")? }
        };
        anyhow::Ok(update)
      })
      .filter_map(|update| match update {
        Ok(update) => Some(update),
        Err(e) => {
          warn!("failed to generate bulk update for alert | {e:#}");
          None
        }
      }).collect::<Vec<_>>();

    bulk_update::bulk_update(
      &db_client().db,
      Alert::default_collection_name(),
      &updates,
      false,
    )
    .await
    .context("failed to bulk update alerts")?;

    anyhow::Ok(())
  };

  let alerts = alerts
    .iter()
    .filter(|(_, send)| *send)
    .map(|(alert, _)| alert)
    .cloned()
    .collect::<Vec<_>>();

  let (res, _) = tokio::join!(open(), send_alerts(&alerts));

  if let Err(e) = res {
    error!("failed to create alerts on db | {e:#}");
  }
}

#[instrument(level = "debug")]
async fn resolve_alerts(alerts: &[(Alert, SendAlerts)]) {
  if alerts.is_empty() {
    return;
  }

  let close = || async move {
    let alert_ids = alerts
      .iter()
      .map(|(alert, _)| {
        ObjectId::from_str(&alert.id)
          .context("failed to convert alert id to ObjectId")
      })
      .collect::<anyhow::Result<Vec<_>>>()?;

    db_client()
      .alerts
      .update_many(
        doc! { "_id": { "$in": &alert_ids } },
        doc! {
          "$set": {
            "resolved": true,
            "resolved_ts": komodo_timestamp()
          }
        },
      )
      .await
      .context("failed to resolve alerts on db")
      .inspect_err(|e| warn!("{e:#}"))
      .ok();

    let ts = komodo_timestamp();

    let closed = alerts
      .iter()
      .filter(|(_, send)| *send)
      .map(|(alert, _)| {
        let mut alert = alert.clone();

        alert.resolved = true;
        alert.resolved_ts = Some(ts);
        alert.level = SeverityLevel::Ok;

        alert
      })
      .collect::<Vec<_>>();

    send_alerts(&closed).await;

    anyhow::Ok(())
  };

  if let Err(e) = close().await {
    error!("failed to resolve alerts | {e:#?}");
  }
}

#[instrument(level = "debug")]
async fn get_open_alerts()
-> anyhow::Result<(OpenAlertMap, OpenDiskAlertMap)> {
  let alerts = find_collect(
    &db_client().alerts,
    doc! { "resolved": false },
    None,
  )
  .await
  .context("failed to get open alerts from db")?;

  let mut map = OpenAlertMap::new();
  let mut disk_map = OpenDiskAlertMap::new();

  for alert in alerts {
    match &alert.data {
      AlertData::ServerDisk { path, .. } => {
        let inner = disk_map.entry(alert.target.clone()).or_default();
        inner.insert(path.to_owned(), alert);
      }
      _ => {
        let inner = map.entry(alert.target.clone()).or_default();
        inner.insert(alert.data.extract_variant(), alert);
      }
    }
  }

  Ok((map, disk_map))
}

/// Alerts should only be opened after
/// 2 *consecutive* alerting conditions.
/// This reduces alerting noise.
#[derive(Default)]
struct AlertBuffer {
  /// (ServerId, AlertType) -> should_open.
  buffer: Mutex<HashMap<(String, AlertDataVariant), bool>>,
}

impl AlertBuffer {
  fn reset(&self, server_id: String, variant: AlertDataVariant) {
    let mut lock = self.buffer.lock().unwrap();
    lock.remove(&(server_id, variant));
  }

  fn ready_to_open(
    &self,
    server_id: String,
    variant: AlertDataVariant,
  ) -> bool {
    let mut lock = self.buffer.lock().unwrap();
    let ready = lock.entry((server_id, variant)).or_default();
    if *ready {
      *ready = false;
      true
    } else {
      *ready = true;
      false
    }
  }
}

fn alert_buffer() -> &'static AlertBuffer {
  static ALERT_BUFFER: OnceLock<AlertBuffer> = OnceLock::new();
  ALERT_BUFFER.get_or_init(Default::default)
}
