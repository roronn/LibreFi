import { memo, useEffect, useState } from "react";
import type { UserType } from "../types/user";

type NumberTickerProps = {
    seconds: number;
    className?: string;
    showMonths?: boolean;
    showDays?: boolean;
    showHours?: boolean;
};

// Plain text avoids animated digit components and their extra layout work.
function NumberTicker({
    seconds,
    className,
    showMonths = true,
    showDays = true,
    showHours = true,
}: NumberTickerProps) {
    const safeSeconds = Math.max(0, Math.floor(seconds));
    const mo = Math.floor(safeSeconds / 2592000);
    const d = Math.floor((safeSeconds % 2592000) / 86400);
    const h = Math.floor((safeSeconds % 86400) / 3600);
    const m = Math.floor((safeSeconds % 3600) / 60);
    const s = safeSeconds % 60;

    const displayMonths = showMonths && mo > 0;
    const displayDays = showDays && (displayMonths || d > 0);
    const displayHours = showHours && (displayDays || h > 0);
    const segmentsCount = 2 + Number(displayHours) + Number(displayDays) + Number(displayMonths);
    const sizeClass = segmentsCount === 5
        ? "text-[11cqw]"
        : segmentsCount === 4
            ? "text-[14cqw]"
            : segmentsCount === 3
                ? "text-[19cqw]"
                : "text-[29cqw]";

    const parts: { value: number; label: string; tone: string }[] = [];
    if (displayMonths) parts.push({ value: mo, label: "MONTH", tone: "text-primary" });
    if (displayDays) parts.push({ value: d, label: "DAY", tone: displayMonths ? "text-foreground/90" : "text-primary/95" });
    if (displayHours) parts.push({ value: h, label: "HOUR", tone: displayDays ? "text-foreground/90" : "text-primary/95" });
    parts.push({ value: m, label: "MIN", tone: displayHours ? "text-foreground/90" : "text-primary/95" });
    parts.push({ value: s, label: "SEC", tone: "text-foreground/70" });

    return (
        <div className={`${className ?? ""} ${sizeClass} flex items-center justify-center gap-[1.5cqw] w-full`}>
            {parts.map((part, index) => (
                <div key={part.label} className="flex items-center gap-[1.5cqw]">
                    {index > 0 && <span aria-hidden="true" className="text-foreground/10">:</span>}
                    <div className="min-w-0 text-center">
                        <span className={`${part.tone} tabular-nums`}>{String(part.value).padStart(2, "0")}</span>
                        <div className="mt-[-2cqw] text-[3cqw] text-foreground/50 text-center pt-[1cqw] tracking-normal w-full">{part.label}</div>
                    </div>
                </div>
            ))}
        </div>
    );
}

function Timer({ user }: { user: UserType | undefined }) {
    const [sec, setSec] = useState(0);

    // Recalculate only when the server-provided session baseline changes.
    useEffect(() => {
        if (!user) {
            setSec(0);
            return;
        }
        setSec(Math.max(0, user.expires_on - (user.paused ? user.paused_on : user.now)));
    }, [user?.expires_on, user?.now, user?.paused, user?.paused_on]);

    // The one-second tick updates only this component's local state.
    useEffect(() => {
        if (!user || user.paused) return;
        const intervalId = window.setInterval(() => {
            setSec(previous => previous > 0 ? previous - 1 : 0);
        }, 1000);
        return () => window.clearInterval(intervalId);
    }, [user?.paused, user?.expires_on]);

    return (
        <div className="@container w-full flex justify-center">
            <NumberTicker seconds={sec} className="text-foreground font-semibold tabular-nums tracking-tighter" />
        </div>
    );
}

export default memo(Timer);
