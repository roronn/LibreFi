import { useRef, useState, type ReactElement } from "react";
import {
    Dialog,
    DialogTrigger,
    DialogContent,
    DialogHeader,
    DialogTitle,
    DialogDescription
} from "../components/ui/dialog";
import CircularProgress from "./ui/circleProgress";
import { Button } from "./ui/button";
import { toast } from "./ui/toast";

function Progress({ timeLeft, totalCoins }: { timeLeft: number; totalCoins: number }) {
    const progress = Math.max(0, Math.min(100, (timeLeft / 30) * 100));

    return (<>
        <div className="relative flex justify-center w-full">
            <CircularProgress progressClassName={`opacity-60 ${progress <= 25 ? "stroke-yellow-400" : "stroke-primary"}`} size={250} strokeWidth={15} value={progress} />
            <div className="w-full h-full grid place-items-center absolute top-0 left-0">
                <h1 className="font-medium text-5xl text-foreground/70">â‚±{totalCoins}</h1>
            </div>
        </div>
    </>)
}

function Coin({ children, onClose }: { children: ReactElement, onClose?: () => void }) {
    const [isOpen, setIsOpen] = useState(false);
    const [timeLeft, setTimeLeft] = useState(30);
    const [totalCoins, setTotalCoins] = useState(0);
    const [lastVoucher, setLastVoucher] = useState("");
    const [recoveryStatus, setRecoveryStatus] = useState<boolean | null>(null);
    const [isCheckingRecovery, setIsCheckingRecovery] = useState(false);
    const wsRef = useRef<WebSocket | null>(null);
    const pingRef = useRef<number | null>(null);
    const timerRef = useRef<number | null>(null);

    function closeModal() {
        setIsOpen(false);
        if (wsRef.current) {
            if (wsRef.current.readyState === WebSocket.OPEN) {
                wsRef.current.send(JSON.stringify({ type: "DONE" }));
            }
            wsRef.current.close();
            wsRef.current = null;
        }
        if (pingRef.current) {
            clearInterval(pingRef.current);
            pingRef.current = null;
        }
        if (timerRef.current) {
            clearInterval(timerRef.current);
            timerRef.current = null;
        }
        setTotalCoins(0);
        onClose?.();
    }

    async function loadLastVoucher() {
        const savedVoucher = localStorage.getItem("librefi_last_voucher") || "";
        setLastVoucher(savedVoucher);
        setRecoveryStatus(null);

        if (!savedVoucher) return;

        setIsCheckingRecovery(true);
        try {
            const response = await fetch("/api/voucher/recovery-status", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ code: savedVoucher })
            });

            if (response.ok) {
                const data = await response.json();
                if (typeof data.recoverable === "boolean") {
                    setRecoveryStatus(data.recoverable);
                }
            }
        } catch (error) {
            console.error("Failed to check last voucher recovery status", error);
        } finally {
            setIsCheckingRecovery(false);
        }
    }

    function handleCoin() {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${protocol}//${window.location.host}/ws`;
        const ws = new WebSocket(wsUrl);
        wsRef.current = ws;

        const loadingToast = toast.loading("Contacting coinslot...", {
            position: "top-center"
        });

        const timeoutId = setTimeout(() => {
            toast.dismiss(loadingToast);
            toast.error("Coinslot is not available!", {
                description: "Cannot contact coinslot within 6 seconds.",
                position: "top-center"
            });
            ws.close();
            wsRef.current = null;
        }, 6000);

        ws.onopen = () => {
            const browserToken = localStorage.getItem("librefi_browser_token");
            ws.send(JSON.stringify({
                type: "ACK",
                value: browserToken
            }));
        };

        ws.onmessage = (event) => {
            console.log("Received WS message:", event.data);
            try {
                const data = JSON.parse(event.data);
                if (data.error) {
                    clearTimeout(timeoutId);
                    toast.dismiss(loadingToast);
                    toast.error("Coinslot is not available!", {
                        description: data.error,
                        position: "top-center"
                    });
                    ws.close();
                } else if (data.ok || data.status === "ok" || data.type === "ACK_SUCCESS") {
                    clearTimeout(timeoutId);
                    toast.dismiss(loadingToast);
                    void loadLastVoucher();
                    setIsOpen(true);
                    setTimeLeft(29);

                    if (timerRef.current) clearInterval(timerRef.current);
                    timerRef.current = window.setInterval(() => {
                        setTimeLeft(prev => {
                            if (prev <= 0.1) {
                                if (timerRef.current) clearInterval(timerRef.current);
                                setTimeout(closeModal, 0);
                                return 0;
                            }
                            return prev - 0.1;
                        });
                    }, 100);

                    if (!pingRef.current) {
                        pingRef.current = window.setInterval(() => {
                            if (ws.readyState === WebSocket.OPEN) {
                                ws.send(JSON.stringify({ type: "ping" }));
                            }
                        }, 500);
                    }
                } else if (data.type === "notify") {
                    setTimeLeft(29);
                } else if (data.type === "timer") {
                    // Ignore backend timer to prefer local smooth timer
                    // const secs = Number(data.value) || 0;
                    // setTimeLeft(secs);
                    // if (secs <= 0) closeModal();
                } else if (data.type === "coin") {
                    let amount = data.value || 0;
                    let seconds = data.time || 0;

                    if (
                        typeof data.browser_token === "string" &&
                        /^[0-9a-fA-F]{64}$/.test(data.browser_token)
                    ) {
                        localStorage.setItem("librefi_browser_token", data.browser_token);
                    }

                    let timeStr = "";
                    if (seconds >= 3600) {
                        timeStr += Math.floor(seconds / 3600) + "h ";
                        seconds %= 3600;
                    }
                    if (seconds >= 60) {
                        timeStr += Math.floor(seconds / 60) + "m ";
                        seconds %= 60;
                    }
                    if (seconds > 0 || timeStr === "") {
                        timeStr += seconds + "s";
                    }

                    setTotalCoins(prev => prev + amount);
                    toast.success("Coin Inserted", {
                        description: `Added ${amount} pesos (${timeStr.trim()})`,
                        position: "top-center"
                    });
                }
            } catch (e) {
                console.error("Failed to parse message", e);
            }
        };

        ws.onerror = () => {
            clearTimeout(timeoutId);
            toast.dismiss(loadingToast);
            toast.error("Coinslot is not available!", {
                description: "Failed to connect to the WebSocket server",
                position: "top-center"
            });
        };
    }

    return (
        <Dialog open={isOpen}>
            <DialogTrigger onClick={handleCoin} render={children}>
                Subscribe
            </DialogTrigger>
            <DialogContent>
                <DialogHeader>
                    <span className="absolute right-2 top-2 block h-6 w-6 bg-popover z-40" />
                    <DialogTitle>Insert Coins</DialogTitle>
                    <DialogDescription>
                        Click done after inserting coins.
                    </DialogDescription>
                </DialogHeader>
                <div className="flex flex-col gap-3">
                    <Progress timeLeft={timeLeft} totalCoins={totalCoins} />

                    {lastVoucher && (
                        <div className="rounded-md border p-3 text-sm">
                            <p className="font-medium">Your last voucher</p>
                            <p className="font-mono text-base">{lastVoucher}</p>
                            {isCheckingRecovery ? (
                                <p className="mt-2 text-muted-foreground">
                                    Checking voucher recovery status...
                                </p>
                            ) : recoveryStatus === true ? (
                                <p className="mt-2">
                                    Your last voucher is still recoverable. Keep it safe! You can use this code to recover your session if you lose your connection or need to log in.
                                </p>
                            ) : recoveryStatus === false ? (
                                <p className="mt-2">
                                    Your previous voucher could not be recovered. Your session has expired. Please purchase a new voucher or insert coins.
                                </p>
                            ) : null}
                        </div>
                    )}

                    <Button onClick={closeModal} className="w-full cursor-pointer hover:bg-primary/80" >
                        Done
                    </Button>
                </div>
            </DialogContent>
        </Dialog>
    )
}

export default Coin;
