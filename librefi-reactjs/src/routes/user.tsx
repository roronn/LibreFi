import Coin from "#components/Coin";
import More, { Price } from "#components/More";
import Timer from "#components/Timer";
import { Button } from "#components/ui/button";
import Voucher from "#components/Voucher";
import {
    CoinsIcon,
    EllipsisVertical,
    PhilippinePeso,
    Loader2Icon,
    PauseIcon,
    PlayIcon,
    Ticket
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
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
            setUser(json);
        }
    }, []);

    const testConnection = useCallback(async () => {
        try {
            setStatus("Testing");

            const res = await fetch(
                `https://dns.google/resolve?name=google.com&_=${Date.now()}`,
                {
                    method: "GET",
                    signal: AbortSignal.timeout(2000)
                }
            );

            if (res.status === 200) {
                setStatus("Connected");

                const hasInternet =
                    user && user.expires_on > user.now && !user.paused;

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
        init();
    }, [init]);

    useEffect(() => {
        (async () => {
            if (user && user.paused) {
                setStatus("Paused");
                return;
            }

            if (user) await testConnection();
        })();
    }, [user]);

    async function handlePlayPause() {
        const playPauseRequest = async () => {
            const res = await fetch("/api/play_pause", {
                method: "POST"
            });

            if (!res.ok) {
                let serverErrorMessage = "Failed to update status";

                try {
                    const errorData = await res.json();
                    serverErrorMessage =
                        errorData.error || serverErrorMessage;
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
                    description:
                        err instanceof Error
                            ? err.message
                            : "Something went wrong."
                })
            },
            { position: "top-center" }
        );
    }

    function updateUser(expires_on: number, now: number) {
        if (user) {
            setUser({ ...user, expires_on, now });
        }
    }

    return (
        <div className="grid place-items-center min-h-screen [@supports(height:100dvh)]:min-h-dvh w-screen">
            <main className="max-w-lg w-full px-6 py-8">
            <header className="text-center mb-8">
                <h1 className="text-2xl font-extrabold tracking-tight">
                    <span className="text-3xl text-yellow-400 font-black tracking-[-0.22em]">//</span><span>NetSn<span className="text-3xl font-extrabold tracking-tight">ꝍ</span>p</span><span className="ml-1">PisoWifi</span>
                </h1>

                    <div className="mt-3 flex items-center justify-center gap-2 text-sm text-foreground/70">
                        {status === "Testing" ? (
                            <Loader2Icon size={14} className="animate-spin" />
                        ) : (
                            <span
                                className={`h-2 w-2 rounded-full ${
                                    status === "Connected"
                                        ? "bg-green-500"
                                        : status === "Paused"
                                          ? "bg-yellow-400"
                                          : "bg-red-500"
                                }`}
                            />
                        )}
                        <span>{status}</span>
                    </div>
                </header>

                <section className="bg-amber-50/30 dark:bg-amber-300/5 rounded-2xl">
                    <div className="w-full flex justify-center backdrop-blur-3xl py-5">
                        <Timer user={user} />
                    </div>
                </section>

                <div className="mt-6">
                    <Coin onClose={init}>
                        <Button
                            className="w-full h-14 rounded-xl text-base font-bold tracking-wide shadow-sm"
                        >
                            <CoinsIcon className="mr-2" size={21} />
                            INSERT COIN
                        </Button>
                    </Coin>
                </div>

                <nav className="w-full flex items-start justify-center gap-8 mt-8">
                    <div className="flex flex-col items-center">
                        <Button
                            onClick={handlePlayPause}
                            variant="outline"
                            size="icon-lg"
                            className="rounded-xl"
                        >
                            {user?.paused ? (
                                <PlayIcon className="text-green-500" />
                            ) : (
                                <PauseIcon className="text-yellow-400" />
                            )}
                        </Button>
                        <span className="text-[0.7rem] text-center mt-2 text-foreground/60">
                            {user?.paused ? "PLAY" : "PAUSE"}
                        </span>
                    </div>

                    <div className="flex flex-col items-center">
                        <Voucher updateUser={updateUser}>
                            <Button
                                variant="outline"
                                size="icon-lg"
                                className="rounded-xl"
                            >
                                <Ticket />
                            </Button>
                        </Voucher>
                        <span className="text-[0.7rem] text-center mt-2 text-foreground/60">
                            VOUCHER
                        </span>
                    </div>

                    <div className="flex flex-col items-center">
                        <Price>
                            <Button
                                variant="outline"
                                size="icon-lg"
                                className="rounded-xl"
                            >
                                <PhilippinePeso />
                            </Button>
                        </Price>
                        <span className="text-[0.7rem] text-center mt-2 text-foreground/60">
                            PRICE
                        </span>
                    </div>

                    <div className="flex flex-col items-center">
                        <More user={user}>
                            <Button
                                variant="outline"
                                size="icon-lg"
                                className="rounded-xl"
                            >
                                <EllipsisVertical />
                            </Button>
                        </More>
                        <span className="text-[0.7rem] text-center mt-2 text-foreground/60">
                            MORE
                        </span>
                    </div>
                </nav>
                <footer className="text-center mt-8 text-xs text-foreground/50">
                    ©2026 LibreFi by @nel003
                </footer>
            </main>
        </div>
    );
}

export default User;
