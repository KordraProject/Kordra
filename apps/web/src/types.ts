export type Channel = "stable" | "nightly" | "master";
export type Download = {
  id: string;
  source: string;
  engine: string;
  status: string;
  downloaded_bytes: number;
  uploaded_bytes: number;
  total_bytes: number;
  error: string | null;
  created_at: string;
  updated_at: string;
};
export type Stats = {
  total_downloaded_bytes: number;
  total_uploaded_bytes: number;
  total_tasks: number;
  completed_tasks: number;
  failed_tasks: number;
  active_tasks: number;
};
export type EngineState = {
  channel: Channel;
  active: string | null;
  fallback: string | null;
  pinned: string | null;
  auto_update: boolean;
  installed: string[];
};

export type DailyTransfer = { day: string; downloaded_bytes: number; uploaded_bytes: number };
export type EngineTransfer = { engine: string; downloaded_bytes: number; total_tasks: number; completed_tasks: number };

export type TorrentDetails = {
  id: number;
  name: string | null;
  info_hash: string;
  paused: boolean;
  output_folder: string;
  only_files: number[] | null;
  trackers: string[];
  stats: {
    progress_bytes: number;
    uploaded_bytes: number;
    total_bytes: number;
    finished: boolean;
    error: string | null;
    file_progress: number[];
  };
};

export type DenoState = {
  active: string | null;
  fallback: string | null;
  pinned: string | null;
  installed: string[];
  auto_update: boolean;
};

export type StartupMode = "hot" | "cold";
export type ColdRetention = "temporary" | "persistent";
export type EngineLifecycleStatus = {
  policy: { startup: StartupMode; cold_retention: ColdRetention };
  engines: Array<{
    id: string;
    kind: string;
    ready: boolean;
    active_tasks?: number;
    active_version?: string | null;
    persistent_process_supported?: boolean;
  }>;
};
