export type Status = 'stopped' | 'connecting' | 'running' | 'retrying' | 'error';
export type Mode = 'Local' | 'Remote' | 'Dynamic';
export interface Host { id: string; name: string; hostname: string; ssh_port: number; username: string; auth_method: 'Password' | 'Key' | 'Agent' | 'Auto'; identity_file: string; jump_chain: string[]; keepalive_seconds: number; has_password: boolean }
export interface HostInput { profile: Host; password: string | null; clear_password: boolean }
export interface Tunnel { id: string; name: string; kind: Mode; host_id: string; listen_address: string; listen_port: number; target_host: string; target_port: number; auto_start: boolean }
export interface RuntimeTunnel { id: string; status: Status; last_error: string | null; next_retry_at: number | null }
export interface Snapshot { tunnels: RuntimeTunnel[] }
export interface Settings { language: import('./i18n').Language; auto_reconnect: boolean; launch_at_login: boolean; close_to_tray: boolean }
export interface LogEntry { id: string; timestamp: string; level: string; tunnel_id: string | null; message: string }
export interface HostKeyPrompt { id: string; host_id: string; hostname: string; port: number; key_type: string; fingerprint: string; old_fingerprint: string | null }
export interface KeyPassphrasePrompt { id: string; host_id: string; host_name: string; identity_file: string; error: string | null }
export interface HostTestResult { elapsed_ms: number; message: string }
export interface TrafficSample { attachment: string; sampled_ms: number; elapsed_ms: number; sent_bytes: string; received_bytes: string }
export const endpoint = (host: string, port: number) => `${host.includes(':') ? `[${host}]` : host}:${port}`;
export const errorText = (e: unknown) => e instanceof Error ? e.message : String(e);
