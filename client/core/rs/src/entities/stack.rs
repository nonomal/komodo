use std::{collections::HashMap, sync::OnceLock};

use bson::{Document, doc};
use derive_builder::Builder;
use derive_default_builder::DefaultBuilder;
use partial_derive2::Partial;
use serde::{Deserialize, Serialize};
use strum::Display;
use typeshare::typeshare;

use crate::deserializers::{
  env_vars_deserializer, file_contents_deserializer,
  option_env_vars_deserializer, option_file_contents_deserializer,
  option_maybe_string_i64_deserializer,
  option_string_list_deserializer, string_list_deserializer,
};

use super::{
  FileContents, SystemCommand,
  docker::container::ContainerListItem,
  resource::{Resource, ResourceListItem, ResourceQuery},
  to_docker_compatible_name,
};

#[typeshare]
pub type Stack = Resource<StackConfig, StackInfo>;

impl Stack {
  /// If fresh is passed, it will bypass the deployed project name.
  /// and get the most up to date one from just project_name field falling back to stack name.
  pub fn project_name(&self, fresh: bool) -> String {
    if !fresh {
      if let Some(project_name) = &self.info.deployed_project_name {
        return project_name.clone();
      }
    }
    self
      .config
      .project_name
      .is_empty()
      .then(|| to_docker_compatible_name(&self.name))
      .unwrap_or_else(|| {
        to_docker_compatible_name(&self.config.project_name)
      })
  }

  pub fn file_paths(&self) -> &[String] {
    if self.config.file_paths.is_empty() {
      default_stack_file_paths()
    } else {
      &self.config.file_paths
    }
  }
}

fn default_stack_file_paths() -> &'static [String] {
  static DEFAULT_FILE_PATHS: OnceLock<Vec<String>> = OnceLock::new();
  DEFAULT_FILE_PATHS
    .get_or_init(|| vec![String::from("compose.yaml")])
}

#[typeshare]
pub type StackListItem = ResourceListItem<StackListItemInfo>;

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackListItemInfo {
  /// The server that stack is deployed on.
  pub server_id: String,
  /// Whether stack is using files on host mode
  pub files_on_host: bool,
  /// Whether stack has file contents defined.
  pub file_contents: bool,
  /// The git provider domain
  pub git_provider: String,
  /// The configured repo
  pub repo: String,
  /// The configured branch
  pub branch: String,
  /// Full link to the repo.
  pub repo_link: String,
  /// The stack state
  pub state: StackState,
  /// A string given by docker conveying the status of the stack.
  pub status: Option<String>,
  /// The services that are part of the stack.
  /// If deployed, will be `deployed_services`.
  /// Otherwise, its `latest_services`
  pub services: Vec<StackServiceWithUpdate>,
  /// Whether the compose project is missing on the host.
  /// Ie, it does not show up in `docker compose ls`.
  /// If true, and the stack is not Down, this is an unhealthy state.
  pub project_missing: bool,
  /// If any compose files are missing in the repo, the path will be here.
  /// If there are paths here, this is an unhealthy state, and deploying will fail.
  pub missing_files: Vec<String>,
  /// Deployed short commit hash, or null. Only for repo based stacks.
  pub deployed_hash: Option<String>,
  /// Latest short commit hash, or null. Only for repo based stacks
  pub latest_hash: Option<String>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackServiceWithUpdate {
  pub service: String,
  /// The service's image
  pub image: String,
  /// Whether there is a newer image available for this service
  pub update_available: bool,
}

#[typeshare]
#[derive(
  Debug,
  Clone,
  Copy,
  Default,
  PartialEq,
  Eq,
  Serialize,
  Deserialize,
  Display,
)]
// Do this one snake_case in line with DeploymentState.
// Also in line with docker terminology.
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StackState {
  /// All containers are running.
  Running,
  /// All containers are paused
  Paused,
  /// All contianers are stopped
  Stopped,
  /// All containers are created
  Created,
  /// All containers are restarting
  Restarting,
  /// All containers are dead
  Dead,
  /// All containers are removing
  Removing,
  /// The containers are in a mix of states
  Unhealthy,
  /// The stack is not deployed
  Down,
  /// Server not reachable
  #[default]
  Unknown,
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StackInfo {
  /// If any of the expected files are missing in the repo,
  /// they will be stored here.
  #[serde(default)]
  pub missing_files: Vec<String>,

  /// The deployed project name.
  /// This is updated whenever Komodo successfully deploys the stack.
  /// If it is present, Komodo will use it for actions over other options,
  /// to ensure control is maintained after changing the project name (there is no rename compose project api).
  pub deployed_project_name: Option<String>,

  /// Deployed short commit hash, or null. Only for repo based stacks.
  pub deployed_hash: Option<String>,
  /// Deployed commit message, or null. Only for repo based stacks
  pub deployed_message: Option<String>,
  /// The deployed compose file contents.
  /// This is updated whenever Komodo successfully deploys the stack.
  pub deployed_contents: Option<Vec<FileContents>>,
  /// The deployed service names.
  /// This is updated whenever it is empty, or deployed contents is updated.
  pub deployed_services: Option<Vec<StackServiceNames>>,
  /// The output of `docker compose config`.
  /// This is updated whenever Komodo successfully deploys the stack.
  pub deployed_config: Option<String>,
  /// The latest service names.
  /// This is updated whenever the stack cache refreshes, using the latest file contents (either db defined or remote).
  #[serde(default)]
  pub latest_services: Vec<StackServiceNames>,

  /// The remote compose file contents, whether on host or in repo.
  /// This is updated whenever Komodo refreshes the stack cache.
  /// It will be empty if the file is defined directly in the stack config.
  pub remote_contents: Option<Vec<FileContents>>,
  /// If there was an error in getting the remote contents, it will be here.
  pub remote_errors: Option<Vec<FileContents>>,

  /// Latest commit hash, or null
  pub latest_hash: Option<String>,
  /// Latest commit message, or null
  pub latest_message: Option<String>,
}

#[typeshare(serialized_as = "Partial<StackConfig>")]
pub type _PartialStackConfig = PartialStackConfig;

/// The compose file configuration.
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Builder, Partial)]
#[partial_derive(Debug, Clone, Default, Serialize, Deserialize)]
#[partial(skip_serializing_none, from, diff)]
pub struct StackConfig {
  /// The server to deploy the stack on.
  #[serde(default, alias = "server")]
  #[partial_attr(serde(alias = "server"))]
  #[builder(default)]
  pub server_id: String,

  /// Configure quick links that are displayed in the resource header
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub links: Vec<String>,

  /// Optionally specify a custom project name for the stack.
  /// If this is empty string, it will default to the stack name.
  /// Used with `docker compose -p {project_name}`.
  ///
  /// Note. Can be used to import pre-existing stacks.
  #[serde(default)]
  #[builder(default)]
  pub project_name: String,

  /// Whether to automatically `compose pull` before redeploying stack.
  /// Ensured latest images are deployed.
  /// Will fail if the compose file specifies a locally build image.
  #[serde(default = "default_auto_pull")]
  #[builder(default = "default_auto_pull()")]
  #[partial_default(default_auto_pull())]
  pub auto_pull: bool,

  /// Whether to `docker compose build` before `compose down` / `compose up`.
  /// Combine with build_extra_args for custom behaviors.
  #[serde(default)]
  #[builder(default)]
  pub run_build: bool,

  /// Whether to poll for any updates to the images.
  #[serde(default)]
  #[builder(default)]
  pub poll_for_updates: bool,

  /// Whether to automatically redeploy when
  /// newer images are found. Will implicitly
  /// enable `poll_for_updates`, you don't need to
  /// enable both.
  #[serde(default)]
  #[builder(default)]
  pub auto_update: bool,

  /// If auto update is enabled, Komodo will
  /// by default only update the specific services
  /// with image updates. If this parameter is set to true,
  /// Komodo will redeploy the whole Stack (all services).
  #[serde(default)]
  #[builder(default)]
  pub auto_update_all_services: bool,

  /// Whether to run `docker compose down` before `compose up`.
  #[serde(default)]
  #[builder(default)]
  pub destroy_before_deploy: bool,

  /// Whether to skip secret interpolation into the stack environment variables.
  #[serde(default)]
  #[builder(default)]
  pub skip_secret_interp: bool,

  /// The git provider domain. Default: github.com
  #[serde(default = "default_git_provider")]
  #[builder(default = "default_git_provider()")]
  #[partial_default(default_git_provider())]
  pub git_provider: String,

  /// Whether to use https to clone the repo (versus http). Default: true
  ///
  /// Note. Komodo does not currently support cloning repos via ssh.
  #[serde(default = "default_git_https")]
  #[builder(default = "default_git_https()")]
  #[partial_default(default_git_https())]
  pub git_https: bool,

  /// The git account used to access private repos.
  /// Passing empty string can only clone public repos.
  ///
  /// Note. A token for the account must be available in the core config or the builder server's periphery config
  /// for the configured git provider.
  #[serde(default)]
  #[builder(default)]
  pub git_account: String,

  /// The Github repo used as the source of the build.
  #[serde(default)]
  #[builder(default)]
  pub repo: String,

  /// The branch of the repo.
  #[serde(default = "default_branch")]
  #[builder(default = "default_branch()")]
  #[partial_default(default_branch())]
  pub branch: String,

  /// Optionally set a specific commit hash.
  #[serde(default)]
  #[builder(default)]
  pub commit: String,

  /// By default, the Stack will `git pull` the repo after it is first cloned.
  /// If this option is enabled, the repo folder will be deleted and recloned instead.
  #[serde(default)]
  #[builder(default)]
  pub reclone: bool,

  /// Whether incoming webhooks actually trigger action.
  #[serde(default = "default_webhook_enabled")]
  #[builder(default = "default_webhook_enabled()")]
  #[partial_default(default_webhook_enabled())]
  pub webhook_enabled: bool,

  /// Optionally provide an alternate webhook secret for this stack.
  /// If its an empty string, use the default secret from the config.
  #[serde(default)]
  #[builder(default)]
  pub webhook_secret: String,

  /// By default, the Stack will `DeployStackIfChanged`.
  /// If this option is enabled, will always run `DeployStack` without diffing.
  #[serde(default)]
  #[builder(default)]
  pub webhook_force_deploy: bool,

  /// If this is checked, the stack will source the files on the host.
  /// Use `run_directory` and `file_paths` to specify the path on the host.
  /// This is useful for those who wish to setup their files on the host,
  /// rather than defining the contents in UI or in a git repo.
  #[serde(default)]
  #[builder(default)]
  pub files_on_host: bool,

  /// Directory to change to (`cd`) before running `docker compose up -d`.
  #[serde(default)]
  #[builder(default)]
  pub run_directory: String,

  /// Add paths to compose files, relative to the run path.
  /// If this is empty, will use file `compose.yaml`.
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub file_paths: Vec<String>,

  /// The name of the written environment file before `docker compose up`.
  /// Relative to the run directory root.
  /// Default: .env
  #[serde(default = "default_env_file_path")]
  #[builder(default = "default_env_file_path()")]
  #[partial_default(default_env_file_path())]
  pub env_file_path: String,

  /// Add additional env files to attach with `--env-file`.
  /// Relative to the run directory root.
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub additional_env_files: Vec<String>,

  /// Whether to send StackStateChange alerts for this stack.
  #[serde(default = "default_send_alerts")]
  #[builder(default = "default_send_alerts()")]
  #[partial_default(default_send_alerts())]
  pub send_alerts: bool,

  /// Used with `registry_account` to login to a registry before docker compose up.
  #[serde(default)]
  #[builder(default)]
  pub registry_provider: String,

  /// Used with `registry_provider` to login to a registry before docker compose up.
  #[serde(default)]
  #[builder(default)]
  pub registry_account: String,

  /// The optional command to run before the Stack is deployed.
  #[serde(default)]
  #[builder(default)]
  pub pre_deploy: SystemCommand,

  /// The optional command to run after the Stack is deployed.
  #[serde(default)]
  #[builder(default)]
  pub post_deploy: SystemCommand,

  /// The extra arguments to pass after `docker compose up -d`.
  /// If empty, no extra arguments will be passed.
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub extra_args: Vec<String>,

  /// The extra arguments to pass after `docker compose build`.
  /// If empty, no extra build arguments will be passed.
  /// Only used if `run_build: true`
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub build_extra_args: Vec<String>,

  /// Ignore certain services declared in the compose file when checking
  /// the stack status. For example, an init service might be exited, but the
  /// stack should be healthy. This init service should be in `ignore_services`
  #[serde(default, deserialize_with = "string_list_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_string_list_deserializer"
  ))]
  #[builder(default)]
  pub ignore_services: Vec<String>,

  /// The contents of the file directly, for management in the UI.
  /// If this is empty, it will fall back to checking git config for
  /// repo based compose file.
  /// Supports variable / secret interpolation.
  #[serde(default, deserialize_with = "file_contents_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_file_contents_deserializer"
  ))]
  #[builder(default)]
  pub file_contents: String,

  /// The environment variables passed to the compose file.
  /// They will be written to path defined in env_file_path,
  /// which is given relative to the run directory.
  ///
  /// If it is empty, no file will be written.
  #[serde(default, deserialize_with = "env_vars_deserializer")]
  #[partial_attr(serde(
    default,
    deserialize_with = "option_env_vars_deserializer"
  ))]
  #[builder(default)]
  pub environment: String,
}

impl StackConfig {
  pub fn builder() -> StackConfigBuilder {
    StackConfigBuilder::default()
  }
}

fn default_env_file_path() -> String {
  String::from(".env")
}

fn default_auto_pull() -> bool {
  true
}

fn default_git_provider() -> String {
  String::from("github.com")
}

fn default_git_https() -> bool {
  true
}

fn default_branch() -> String {
  String::from("main")
}

fn default_webhook_enabled() -> bool {
  true
}

fn default_send_alerts() -> bool {
  true
}

impl Default for StackConfig {
  fn default() -> Self {
    Self {
      server_id: Default::default(),
      project_name: Default::default(),
      run_directory: Default::default(),
      file_paths: Default::default(),
      files_on_host: Default::default(),
      registry_provider: Default::default(),
      registry_account: Default::default(),
      file_contents: Default::default(),
      auto_pull: default_auto_pull(),
      poll_for_updates: Default::default(),
      auto_update: Default::default(),
      auto_update_all_services: Default::default(),
      ignore_services: Default::default(),
      pre_deploy: Default::default(),
      post_deploy: Default::default(),
      extra_args: Default::default(),
      environment: Default::default(),
      env_file_path: default_env_file_path(),
      additional_env_files: Default::default(),
      run_build: Default::default(),
      destroy_before_deploy: Default::default(),
      build_extra_args: Default::default(),
      skip_secret_interp: Default::default(),
      git_provider: default_git_provider(),
      git_https: default_git_https(),
      repo: Default::default(),
      branch: default_branch(),
      commit: Default::default(),
      reclone: Default::default(),
      git_account: Default::default(),
      webhook_enabled: default_webhook_enabled(),
      webhook_secret: Default::default(),
      webhook_force_deploy: Default::default(),
      send_alerts: default_send_alerts(),
      links: Default::default(),
    }
  }
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeProject {
  /// The compose project name.
  pub name: String,
  /// The status of the project, as returned by docker.
  pub status: Option<String>,
  /// The compose files included in the project.
  pub compose_files: Vec<String>,
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StackServiceNames {
  /// The name of the service
  pub service_name: String,
  /// Will either be the declared container_name in the compose file,
  /// or a pattern to match auto named containers.
  ///
  /// Auto named containers are composed of three parts:
  ///
  /// 1. The name of the compose project (top level name field of compose file).
  ///    This defaults to the name of the parent folder of the compose file.
  ///    Komodo will always set it to be the name of the stack, but imported stacks
  ///    will have a different name.
  /// 2. The service name
  /// 3. The replica number
  ///
  /// Example: stacko-mongo-1.
  ///
  /// This stores only 1. and 2., ie stacko-mongo.
  /// Containers will be matched via regex like `^container_name-?[0-9]*$``
  pub container_name: String,
  /// The services image.
  #[serde(default)]
  pub image: String,
}

#[typeshare]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StackService {
  /// The service name
  pub service: String,
  /// The service image
  pub image: String,
  /// The container
  pub container: Option<ContainerListItem>,
  /// Whether there is an update available for this services image.
  pub update_available: bool,
}

#[typeshare]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct StackActionState {
  pub pulling: bool,
  pub deploying: bool,
  pub starting: bool,
  pub restarting: bool,
  pub pausing: bool,
  pub unpausing: bool,
  pub stopping: bool,
  pub destroying: bool,
}

#[typeshare]
pub type StackQuery = ResourceQuery<StackQuerySpecifics>;

#[typeshare]
#[derive(
  Serialize, Deserialize, Debug, Clone, Default, DefaultBuilder,
)]
pub struct StackQuerySpecifics {
  /// Query only for Stacks on these Servers.
  /// If empty, does not filter by Server.
  /// Only accepts Server id (not name).
  #[serde(default)]
  pub server_ids: Vec<String>,
  /// Filter syncs by their repo.
  #[serde(default)]
  pub repos: Vec<String>,
  /// Query only for Stack with available image updates.
  #[serde(default)]
  pub update_available: bool,
}

impl super::resource::AddFilters for StackQuerySpecifics {
  fn add_filters(&self, filters: &mut Document) {
    if !self.server_ids.is_empty() {
      filters
        .insert("config.server_id", doc! { "$in": &self.server_ids });
    }
    if !self.repos.is_empty() {
      filters.insert("config.repo", doc! { "$in": &self.repos });
    }
  }
}

/// Keeping this minimal for now as its only needed to parse the service names / container names,
/// and replica count. Not a typeshared type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeFile {
  /// If not provided, will default to the parent folder holding the compose file.
  pub name: Option<String>,
  #[serde(default)]
  pub services: HashMap<String, ComposeService>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeService {
  pub image: Option<String>,
  pub container_name: Option<String>,
  pub deploy: Option<ComposeServiceDeploy>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeServiceDeploy {
  #[serde(
    default,
    deserialize_with = "option_maybe_string_i64_deserializer"
  )]
  pub replicas: Option<i64>,
}
