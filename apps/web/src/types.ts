export type Channel = "stable" | "nightly" | "master";
export type Download = {
  id: string;
  source: string;
  engine: string;
  status: string;
  downloaded_bytes: number;
  uploaded_bytes: number;
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

export type DailyTransfer = { day: string; downloaded_bytes: number };
export type EngineTransfer = { engine: string; downloaded_bytes: number; total_tasks: number; completed_tasks: number };
