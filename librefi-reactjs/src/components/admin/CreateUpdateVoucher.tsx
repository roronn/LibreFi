import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "#components/ui/popover";
import { CheckCircle2, Loader2Icon } from "lucide-react";
import { Label } from "../ui/label";
import { Input } from "../ui/input";
import { toast } from "../ui/toast";
import {
    Dialog,
    DialogContent,
    DialogHeader,
    DialogTitle,
} from "../ui/dialog";
import { useState } from "react";
import { encrypt, formatSeconds, getAdminKey } from "#lib/utils";
import { Button } from "../ui/button";

type Voucher = {
    code: string;
    price: number;
    time: number;
    used: boolean;
};

export default function CreateEditVoucher({
    getVouchers,
    btn,
}: {
    getVouchers: () => Promise<void>;
    btn: React.ReactElement;
}) {
    const [price, setPrice] = useState("20");
    const [time, setTime] = useState("3600");
    const [quantity, setQuantity] = useState("10");
    const [loading, setLoading] = useState(false);
    const [open, setOpen] = useState(false);
    const [dialog, setDialog] = useState(false);
    const [batch, setBatch] = useState<Voucher[]>([]);

    async function generateBatch() {
        const numericPrice = Number(price);
        const numericTime = Number(time);
        const numericQuantity = Number(quantity);

        if (!Number.isFinite(numericPrice) || numericPrice <= 0) {
            toast.error("Invalid price", {
                description: "Enter a price greater than zero.",
                position: "top-center",
            });
            return;
        }

        if (!Number.isInteger(numericTime) || numericTime <= 0) {
            toast.error("Invalid time", {
                description: "Enter a positive whole number of seconds.",
                position: "top-center",
            });
            return;
        }

        if (
            !Number.isInteger(numericQuantity) ||
            numericQuantity < 1 ||
            numericQuantity > 500
        ) {
            toast.error("Invalid quantity", {
                description: "Quantity must be between 1 and 500.",
                position: "top-center",
            });
            return;
        }

        setLoading(true);

        try {
            const encryptedPayload = await encrypt(
                getAdminKey(),
                JSON.stringify({
                    price: numericPrice,
                    time: numericTime,
                    quantity: numericQuantity,
                })
            );

            if (!encryptedPayload) {
                throw new Error("Could not encrypt the voucher request.");
            }

            const res = await fetch("/api/admin/vouchers", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({ payload: encryptedPayload }),
            });

            const json = await res.json();

            if (!res.ok || !json.ok || !Array.isArray(json.vouchers)) {
                throw new Error(json.error || "Voucher generation failed.");
            }

            const generated: Voucher[] = json.vouchers;

            if (generated.length !== numericQuantity) {
                throw new Error("The server returned an unexpected voucher count.");
            }

            setBatch(generated);
            setOpen(false);
            setDialog(true);

            toast.success("Voucher batch generated", {
                description: `${generated.length} vouchers created successfully.`,
                position: "top-center",
            });

            try {
                await getVouchers();
            } catch (refreshError) {
                console.error("Could not refresh voucher list:", refreshError);
            }
        } catch (error) {
            toast.error("Generation failed", {
                description:
                    error instanceof Error
                        ? error.message
                        : "Could not generate vouchers.",
                position: "top-center",
            });
        } finally {
            setLoading(false);
        }
    }


    return (
        <div>
            <Popover open={open} onOpenChange={setOpen}>
                <PopoverTrigger render={btn}>
                    <div aria-hidden="true" />
                </PopoverTrigger>

                <PopoverContent className="w-72 gap-0 p-0" align="end">
                    <div className="border-b p-3">
                        <h4 className="m-0 font-semibold">Generate Voucher Batch</h4>
                        <p className="text-muted-foreground text-sm">
                            Create multiple vouchers at once.
                        </p>
                    </div>

                    <div className="space-y-3 p-3 pb-4">
                        <div className="space-y-1.5">
                            <Label>Price</Label>
                            <Input
                                type="number"
                                min="0.01"
                                step="0.01"
                                value={price}
                                onChange={(e) => setPrice(e.target.value)}
                            />
                        </div>

                        <div className="space-y-1.5">
                            <Label>
                                Time <span className="text-foreground/60">(seconds)</span>
                            </Label>
                            <Input
                                type="number"
                                min="1"
                                step="1"
                                value={time}
                                onChange={(e) => setTime(e.target.value)}
                            />
                            <p className="text-xs text-muted-foreground">
                                {formatSeconds(Number(time) || 0, 0)}
                            </p>
                        </div>

                        <div className="space-y-1.5">
                            <Label>Quantity (1–500)</Label>
                            <Input
                                type="number"
                                min="1"
                                max="500"
                                step="1"
                                value={quantity}
                                onChange={(e) => setQuantity(e.target.value)}
                            />
                        </div>
                    </div>

                    <div className="p-2">
                        <Button
                            onClick={generateBatch}
                            disabled={loading}
                            className="w-full"
                        >
                            Generate Batch
                            {loading ? (
                                <Loader2Icon className="animate-spin" />
                            ) : null}
                        </Button>
                    </div>
                </PopoverContent>
            </Popover>

            <Dialog open={dialog} onOpenChange={setDialog}>
                <DialogContent>
                    <DialogHeader>
                        <DialogTitle className="flex items-center justify-center gap-2 text-primary">
                            <CheckCircle2 size={24} />
                            Batch Generated
                        </DialogTitle>

                        <p className="text-center text-muted-foreground">
                            {batch.length} vouchers were generated successfully.
                        </p>

                        <div className="max-h-52 overflow-y-auto rounded-lg border p-3">
                            {batch.map((voucher) => (
                                <div
                                    key={voucher.code}
                                    className="flex justify-between gap-3 border-b py-1.5 last:border-0"
                                >
                                    <span className="font-mono font-bold">
                                        {voucher.code}
                                    </span>
                                    <span className="text-muted-foreground">
                                        PHP {Number(voucher.price).toFixed(2)}
                                    </span>
                                </div>
                            ))}
                        </div>

                    </DialogHeader>
                </DialogContent>
            </Dialog>
        </div>
    );
}
