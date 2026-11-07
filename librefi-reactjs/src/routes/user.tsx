import Timer from "#components/Timer";
import UserControls from "#components/UserControls";
import { Loader2Icon } from "lucide-react"
import { useCallback, useEffect, useState } from "react"
import type { UserType } from ".././types/user";
import { toast } from "#components/ui/toast";


function User() {
    const [user, setUser] = useState<UserType>();
    const [status, setStatus] = useState("--");

    const init = useCallback(async () => {
        const res = await fetch(`/api/init?_=${Date.now()}`, {
            method: "GET"
        });

        if (res.status === 200) {
            const json = await res.json();
            setUser(json)
        }

    }, []);

    const testConnection = useCallback(async () => {
        try {
            setStatus("Testing");
            const res = await fetch(`https://dns.google/resolve?name=google.com&_=${Date.now()}`, {
                method: "GET",
                signal: AbortSignal.timeout(2000)
            });

            if (res.status === 200) {
                setStatus("Connected");

                const hasInternet = user && (user.expires_on > user.now) && !user.paused;

                if (hasInternet && window.location.pathname !== "/") {
                    const url = new URL(window.location.href);
                    const r = parseInt(url.searchParams.get("r") || "0");

                    if (r >= 2) {
                        window.location.replace("/");
                    } else {
                        url.searchParams.set("r", (r + 1).toString());
                        window.location.replace(url.toString());
                        window.location.reload();
                    }
                    return;
                }
            }
        } catch (error) {
            setStatus("Disconnected");
        }

    }, []);

    useEffect(() => {
        (() => {
            init();
        })()
    }, [init])

    useEffect(() => {
        (async () => {
            if (user && user?.paused) {
                setStatus("Paused")
                return;
            }

            if (user) await testConnection();
        })()
    }, [user])

    const handlePlayPause = useCallback(async () => {
        const playPauseRequest = async () => {
            const res = await fetch("/api/play_pause", {
                method: "POST"
            });

            if (!res.ok) {
                let serverErrorMessage = "Failed to update status";

                try {
                    const errorData = await res.json();
                    serverErrorMessage = errorData.error || serverErrorMessage;
                } catch {
                    const textError = await res.text();
                    if (textError) serverErrorMessage = textError;
                }

                throw new Error(serverErrorMessage);
            }

            const json = await res.json();

            if (user) {
                setUser({
                    ...user,
                    expires_on: json.expires_on,
                    now: json.now,
                    pause_attempts: json.pause_attempts,
                    paused: json.paused,
                    paused_on: json.paused_on
                });
                return json;
            } else {
                throw new Error("User session not found.");
            }
        };

        toast.promise(
            playPauseRequest(),
            {
                loading: {
                    title: "Updating status...",
                    description: "Please wait while we update your connection."
                },
                success: {
                    title: "Success",
                    description: "Status has been updated successfully."
                },
                error: (err) => ({
                    title: "Update failed",
                    description: err instanceof Error ? err.message : "Something went wrong."
                })
            },
            { position: "top-center" }
        );
    }, [user]);

    const updateUser = useCallback((expires_on: number, now: number) => {
        if (user) setUser({ ...user, expires_on, now });
    }, [user]);

    return (
        <>
            <div className="grid place-items-center h-screen [@supports(height:100dvh)]:h-dvh w-screen relative">
                <div className="absolute bottom-0 w-full text-center text-foreground/80">
                    <p className="p-1 text-xs">©{new Date().getFullYear()} LibreFi by <a className="text-foreground underline decoration-primary underline-offset-1">@nel003</a></p>
                </div>

                <div className="max-w-lg w-full p-6">
                    <div className="text-center pb-5 font-extrabold tracking-tight text-3xl"><span className="text-amber-400 text-4xl font-black tracking-[-0.28em] mr-[5px]">//</span>NetSn<span className="text-4xl inline-block">ꝍ</span>p PisoWifi</div>
                    <h1 className="text-center pb-8 text-sm flex items-center justify-center ">{status} {status === "Testing" ? <span className="pl-1 "> <Loader2Icon size={16} className="animate-spin " /></span> : <></>} </h1>

                    <div className="bg-amber-50/30 dark:bg-amber-300/5">
                        <div className="w-full flex justify-center py-4">
                            <Timer
                                user={user}
                            />
                        </div>
                    </div>

                    <UserControls
                        user={user}
                        onRefresh={init}
                        onPlayPause={handlePlayPause}
                        onUpdateUser={updateUser}
                    />

                </div>
            </div>
        </>
    )
}

export default User;