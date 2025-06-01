import { atomWithStorage, useLocalStorage, useRead, useUser } from "@lib/hooks";
import { RequiredResourceComponents } from "@types";
import { Card } from "@ui/card";
import { Clock, FolderSync } from "lucide-react";
import { DeleteResource, NewResource, StandardSource } from "../common";
import { ResourceSyncTable } from "./table";
import { Types } from "komodo_client";
import { CommitSync, ExecuteSync, RefreshSync } from "./actions";
import {
  border_color_class_by_intention,
  resource_sync_state_intention,
  stroke_color_class_by_intention,
} from "@lib/color";
import { cn, sync_no_changes } from "@lib/utils";
import { fmt_date } from "@lib/formatting";
import { DashboardPieChart } from "@pages/home/dashboard";
import { ResourcePageHeader, StatusBadge } from "@components/util";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@ui/tabs";
import { ResourceSyncConfig } from "./config";
import { ResourceSyncInfo } from "./info";
import { ResourceSyncPending } from "./pending";
import { Badge } from "@ui/badge";
import { GroupActions } from "@components/group-actions";
import { useAtom } from "jotai";
import { Tooltip, TooltipContent, TooltipTrigger } from "@ui/tooltip";

export const useResourceSync = (id?: string) =>
  useRead("ListResourceSyncs", {}, { refetchInterval: 10_000 }).data?.find(
    (d) => d.id === id
  );

export const useFullResourceSync = (id: string) =>
  useRead("GetResourceSync", { sync: id }, { refetchInterval: 10_000 }).data;

const ResourceSyncIcon = ({ id, size }: { id?: string; size: number }) => {
  const state = useResourceSync(id)?.info.state;
  const color = stroke_color_class_by_intention(
    resource_sync_state_intention(state)
  );
  return <FolderSync className={cn(`w-${size} h-${size}`, state && color)} />;
};

const pendingViewAtom = atomWithStorage<"Execute" | "Commit">(
  "sync-view-v1",
  "Execute"
);
export const usePendingView = () => {
  return useAtom(pendingViewAtom) as [
    "Execute" | "Commit",
    (view: "Execute" | "Commit") => void,
  ];
};

const ConfigInfoPending = ({ id }: { id: string }) => {
  const [_view, setView] = useLocalStorage<"Config" | "Info" | "Pending">(
    "sync-tabs-v3",
    "Config"
  );
  const sync = useFullResourceSync(id);

  const hideInfo = sync?.config?.files_on_host
    ? false
    : sync?.config?.file_contents
      ? true
      : false;

  const showPending =
    sync && (!sync_no_changes(sync) || sync.info?.pending_error);

  const view =
    _view === "Info" && hideInfo
      ? "Config"
      : _view === "Pending" && !showPending
        ? sync?.config?.files_on_host || sync?.config?.repo
          ? "Info"
          : "Config"
        : _view;

  const title = (
    <TabsList className="justify-start w-fit">
      <TabsTrigger value="Config" className="w-[110px]">
        Config
      </TabsTrigger>
      <TabsTrigger
        value="Info"
        className={cn("w-[110px]", hideInfo && "hidden")}
        disabled={hideInfo}
      >
        Info
      </TabsTrigger>
      <TabsTrigger
        value="Pending"
        className="w-[110px]"
        disabled={!showPending}
      >
        Pending
      </TabsTrigger>
    </TabsList>
  );
  return (
    <Tabs value={view} onValueChange={setView as any} className="grid gap-4">
      <TabsContent value="Config">
        <ResourceSyncConfig id={id} titleOther={title} />
      </TabsContent>
      <TabsContent value="Info">
        <ResourceSyncInfo id={id} titleOther={title} />
      </TabsContent>
      <TabsContent value="Pending">
        <ResourceSyncPending id={id} titleOther={title} />
      </TabsContent>
    </Tabs>
  );
};

export const ResourceSyncComponents: RequiredResourceComponents = {
  list_item: (id) => useResourceSync(id),
  resource_links: () => undefined,

  Description: () => <>Declare resources in TOML files.</>,

  Dashboard: () => {
    const summary = useRead("GetResourceSyncsSummary", {}).data;
    return (
      <DashboardPieChart
        data={[
          { title: "Ok", intention: "Good", value: summary?.ok ?? 0 },
          {
            title: "Syncing",
            intention: "Warning",
            value: summary?.syncing ?? 0,
          },
          {
            title: "Pending",
            intention: "Neutral",
            value: summary?.pending ?? 0,
          },
          {
            title: "Failed",
            intention: "Critical",
            value: summary?.failed ?? 0,
          },
          {
            title: "Unknown",
            intention: "Unknown",
            value: summary?.unknown ?? 0,
          },
        ]}
      />
    );
  },

  New: () => {
    const admin = useUser().data?.admin;
    return (
      admin && <NewResource type="ResourceSync" readable_type="Resource Sync" />
    );
  },

  GroupActions: () => (
    <GroupActions type="ResourceSync" actions={["RunSync"]} />
  ),

  Table: ({ resources }) => (
    <ResourceSyncTable syncs={resources as Types.ResourceSyncListItem[]} />
  ),

  Icon: ({ id }) => <ResourceSyncIcon id={id} size={4} />,
  BigIcon: ({ id }) => <ResourceSyncIcon id={id} size={8} />,

  State: ({ id }) => {
    const state = useResourceSync(id)?.info.state;
    return (
      <StatusBadge text={state} intent={resource_sync_state_intention(state)} />
    );
  },

  Info: {
    Source: ({ id }) => {
      const info = useResourceSync(id)?.info;
      return <StandardSource info={info} />;
    },
    LastSync: ({ id }) => {
      const last_ts = useResourceSync(id)?.info.last_sync_ts;
      return (
        <div className="flex items-center gap-2">
          <Clock className="w-4 h-4" />
          {last_ts ? fmt_date(new Date(last_ts)) : "Never"}
        </div>
      );
    },
  },

  Status: {
    Hash: ({ id }) => {
      const info = useFullResourceSync(id)?.info;
      if (!info?.pending_hash) {
        return null;
      }
      const out_of_date =
        info.last_sync_hash && info.last_sync_hash !== info.pending_hash;
      return (
        <Tooltip>
          <TooltipTrigger asChild>
            <Card
              className={cn(
                "px-3 py-2 hover:bg-accent/50 transition-colors cursor-pointer",
                out_of_date && border_color_class_by_intention("Warning")
              )}
            >
              <div className="text-muted-foreground text-sm text-nowrap overflow-hidden overflow-ellipsis">
                {info.last_sync_hash ? "synced" : "latest"}:{" "}
                {info.last_sync_hash || info.pending_hash}
              </div>
            </Card>
          </TooltipTrigger>
          <TooltipContent>
            <div className="grid gap-2">
              <Badge
                variant="secondary"
                className="w-fit text-muted-foreground"
              >
                message
              </Badge>
              {info.last_sync_message || info.pending_message}
              {out_of_date && (
                <>
                  <Badge
                    variant="secondary"
                    className={cn(
                      "w-fit text-muted-foreground border-[1px]",
                      border_color_class_by_intention("Warning")
                    )}
                  >
                    latest
                  </Badge>
                  <div>
                    <span className="text-muted-foreground">
                      {info.pending_hash}
                    </span>
                    : {info.pending_message}
                  </div>
                </>
              )}
            </div>
          </TooltipContent>
        </Tooltip>
      );
    },
  },

  Actions: { RefreshSync, ExecuteSync, CommitSync },

  Page: {},

  Config: ConfigInfoPending,

  DangerZone: ({ id }) => <DeleteResource type="ResourceSync" id={id} />,

  ResourcePageHeader: ({ id }) => {
    const sync = useResourceSync(id);

    return (
      <ResourcePageHeader
        intent={resource_sync_state_intention(sync?.info.state)}
        icon={<ResourceSyncIcon id={id} size={8} />}
        type="ResourceSync"
        id={id}
        name={sync?.name}
        state={sync?.info.state}
        status=""
      />
    );
  },
};
