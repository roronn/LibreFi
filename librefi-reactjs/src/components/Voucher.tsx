import { useState, type ReactElement } from "react";
import { Input } from "./ui/input";
import { Button } from "./ui/button";
import {
    Dialog,
    DialogTrigger,
    DialogContent,
    DialogHeader,
    DialogTitle,
    DialogDescription
} from "../components/ui/dialog";
import { toast } from "./ui/toast";

function Voucher({
    onRedeemed,
    children
}: {
    onRedeemed: (browserToken?: string, redeemedCode?: string) => void | Promise<void>;
    children: ReactElement;
}) {
    const [isOpen, setIsOpen] = useState(false);
    const [isRedeeming, setIsRedeeming] = useState(false);
    const [code, setCode] = useState("");
    const [lastVoucher, setLastVoucher] = useState("");
    const [recoveryStatus, setRecoveryStatus] = useState<boolean | null>(null);
    const [isCheckingRecovery, setIsCheckingRecovery] = useState(false);

    async function handleOpenChange(open: boolean) {
        setIsOpen(open);

        if (!open) return;

        const savedVoucher = localStorage.getItem("librefi_last_voucher") || "";
        setLastVoucher(savedVoucher);
        setRecoveryStatus(null);

        if (!savedVoucher) return;

        setIsCheckingRecovery(true);

        try {
            const res = await fetch("/api/voucher/recovery-status", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json"
                },
                body: JSON.stringify({ code: savedVoucher })
            });

            if (!res.ok) return;

            const json = await res.json();

            if (typeof json.recoverable === "boolean") {
                setRecoveryStatus(json.recoverable);
            }
        } catch {
            // Do not show an expired-session message when the status check fails.
        } finally {
            setIsCheckingRecovery(false);
        }
    }

    async function handleRedeem() {
        const redeemRequest = async () => {
            setIsRedeeming(true);

            try {
                const normalizedCode = code.replace(/\s/g, "");
                const res = await fetch("/api/redeem", {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json"
                    },
                    body: JSON.stringify({
                        code: normalizedCode,
                        browser_token: localStorage.getItem("librefi_browser_token")
                    })
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

                await onRedeemed(
                    typeof json.browser_token === "string" ? json.browser_token : undefined,
                    normalizedCode
                );

                setCode("");
                setIsOpen(false);
            } finally {
                setIsRedeeming(false);
            }
        };

        toast.promise(
            redeemRequest(),
            {
                loading: {
                    title: "Processing...",
                    description: "Please wait while we process your voucher."
                },
                success: {
                    title: "Success",
                    description: "Voucher applied successfully."
                },
                error: (err) => ({
                    title: "Update failed",
                    description: err instanceof Error ? err.message : "Something went wrong."
                })
            },
            { position: "top-center" }
        );
    }

    return (
        <>
            <Dialog open={isOpen} onOpenChange={handleOpenChange}>
                <DialogTrigger render={children}>
                    Subscribe
                </DialogTrigger>
                <DialogContent>
                    <DialogHeader>
                        <DialogTitle>Redeem Voucher</DialogTitle>
                        <DialogDescription>
                            Enter your voucher and click redeem.
                        </DialogDescription>
                    </DialogHeader>

                    <div className="flex flex-col gap-3">
                        {lastVoucher && (
                            <div className="rounded-md border p-3 text-sm">
                                <p className="font-medium">Your last voucher</p>
                                <p className="mt-1 break-all font-mono">{lastVoucher}</p>

                                {isCheckingRecovery && (
                                    <p className="mt-2 text-muted-foreground">
                                        Checking voucher recovery...
                                    </p>
                                )}

                                {recoveryStatus === true && (
                                    <p className="mt-2">
                                        Your last voucher is still recoverable. Keep it safe! You can use this code to recover your session if you lose your connection or need to log in.
                                    </p>
                                )}

                                {recoveryStatus === false && (
                                    <p className="mt-2">
                                        This voucher is not currently eligible for recovery. If you disconnected recently, wait until the seven-minute grace period has passed. If your session expired, purchase a new voucher or insert coins.
                                    </p>
                                )}
                            </div>
                        )}

                        <Input
                            onChange={(e) => setCode(e.target.value.replace(/\s/g, ""))}
                            value={code}
                            type="text"
                            placeholder="E.g xrm123"
                            className="h-9"
                        />

                        <Button
                            disabled={isRedeeming || !code.trim()}
                            onClick={handleRedeem}
                            className="w-full cursor-pointer hover:bg-primary/80"
                        >
                            Redeem Now
                        </Button>

                        <Button
                            onClick={() => setIsOpen(false)}
                            variant="ghost"
                            className="w-full cursor-pointer -mt-2"
                        >
                            Cancel
                        </Button>
                    </div>
                </DialogContent>
            </Dialog>
        </>
    );
}

export default Voucher;