export async function api<T>(path: string, token: string, options?: RequestInit): Promise<T> {
  const headers = new Headers(options?.headers);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  if (options?.body) headers.set("Content-Type", "application/json");
  const response = await fetch(`/api/v1${path}`, { ...options, headers });
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(`HTTP ${response.status}: ${detail.slice(0, 400)}`);
  }
  return (await response.json()) as T;
}
export function post<T>(path: string, token: string, body?: object): Promise<T> {
  return api<T>(path, token, { method: "POST", body: JSON.stringify(body ?? {}) });
}
export function prettyBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  const power = Math.min(Math.floor(Math.log(value) / Math.log(1024)), 5);
  return `${(value / 1024 ** power).toFixed(2)} ${units[power - 1]}`;
}
