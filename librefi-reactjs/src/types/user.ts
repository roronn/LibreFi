export interface UserType {
    session_id: string;
    browser_token: string;
    remaining_seconds: number;
    ip: string;
    mac: string;
    name: string;
    now: number;
    pause_attempts: number;
    pause_day: number;
    paused: boolean;
    status: string;
}
