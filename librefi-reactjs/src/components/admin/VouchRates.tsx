import { encrypt, formatSeconds, getAdminKey } from "#lib/utils";
import { useEffect, useState } from "react";
import {
    Table,
    TableBody,
    TableCaption,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "../ui/table";
import { Button } from "../ui/button";
import { useLocation } from "wouter";
import { Download, Plus } from "lucide-react";
import { jsPDF } from "jspdf";
import type { RateType } from "../../types/rates";
import CreateEditVoucher from "./CreateUpdateVoucher";
import DeleteVoucher from "./DeleteVoucher";
import CreateUpdateRate from "./CreateUpdateRate";
import DeleteRate from "./DeleteRate";
import { toast } from "../ui/toast";

interface VoucherT {
    code: string;
    price: number;
    time: number;
    used: boolean;
    censored: boolean;
    pdf_download_count: number;
}

function censor(code: string) {
    return code.substring(0, 2) + "****";
}

function toRoman(value: number) {
    const numerals: [number, string][] = [
        [1000, "M"], [900, "CM"], [500, "D"], [400, "CD"],
        [100, "C"], [90, "XC"], [50, "L"], [40, "XL"],
        [10, "X"], [9, "IX"], [5, "V"], [4, "IV"], [1, "I"],
    ];

    let number = Math.max(1, Math.floor(value));
    let result = "";

    for (const [amount, symbol] of numerals) {
        while (number >= amount) {
            result += symbol;
            number -= amount;
        }
    }

    return result;
}

function AdminVouchRates() {
    const [vouchers, setVouchers] = useState<VoucherT[] | null>();
    const [rates, setRates] = useState<RateType[] | null>();
    const [voucherPage, setVoucherPage] = useState(1);
    const [voucherTotalPages, setVoucherTotalPages] = useState(1);
    const [selectedVouchers, setSelectedVouchers] = useState<VoucherT[]>([]);
    const [downloading, setDownloading] = useState(false);

    const [_, setLocation] = useLocation();

    async function getVouchers() {
        const encryptedPayload = await encrypt(
            getAdminKey(),
            JSON.stringify({ page: voucherPage })
        );

        const res = await fetch(
            "/api/admin/vouchers?payload=" + encodeURIComponent(encryptedPayload || "")
        );

        if (res.ok) {
            const json = await res.json();
            setVouchers(
                json.vouchers.map((v: VoucherT) => ({
                    ...v,
                    censored: true,
                    pdf_download_count: v.pdf_download_count ?? 0,
                }))
            );
            setVoucherTotalPages(json.total_pages);
        } else {
            setLocation("/");
        }
    }

    async function getRates() {
        const res = await fetch("/api/rates");
        if (res.ok) {
            const json = await res.json();
            setRates(json.rows);
        }
    }

    useEffect(() => {
        getVouchers();
    }, [voucherPage]);

    useEffect(() => {
        getRates();
    }, []);

    function toggleVoucher(voucher: VoucherT, checked: boolean) {
        if (voucher.used) return;

        setSelectedVouchers((current) => {
            if (checked) {
                if (current.some((item) => item.code === voucher.code)) return current;
                return [...current, voucher];
            }

            return current.filter((item) => item.code !== voucher.code);
        });
    }

    function selectAllUnused(checked: boolean) {
        const unused = (vouchers || []).filter((voucher) => !voucher.used);

        setSelectedVouchers((current) => {
            if (checked) {
                const additions = unused.filter(
                    (voucher) => !current.some((item) => item.code === voucher.code)
                );
                return [...current, ...additions];
            }

            const pageCodes = new Set(unused.map((voucher) => voucher.code));
            return current.filter((voucher) => !pageCodes.has(voucher.code));
        });
    }

    async function downloadSelectedPDF() {
        const selected = selectedVouchers.filter((voucher) => !voucher.used);

        if (selected.length === 0 || downloading) return;

        setDownloading(true);

        try {
            const pdf = new jsPDF({
                orientation: "portrait",
                unit: "mm",
                format: "a4",
            });

            const pageWidth = 210;
            const margin = 10;
            const columns = 2;
            const rows = 5;
            const gapX = 6;
            const gapY = 5;
            const voucherWidth = (pageWidth - margin * 2 - gapX) / columns;
            const voucherHeight = 50;

            selected.forEach((voucher, index) => {
                const position = index % (columns * rows);

                if (index > 0 && position === 0) {
                    pdf.addPage();
                }

                const column = position % columns;
                const row = Math.floor(position / columns);
                const x = margin + column * (voucherWidth + gapX);
                const y = margin + row * (voucherHeight + gapY);

                pdf.setDrawColor(130, 130, 130);
                pdf.setLineWidth(0.35);
                pdf.setLineDashPattern([2, 1.5], 0);
                pdf.roundedRect(x, y, voucherWidth, voucherHeight, 2, 2, "S");
                pdf.setLineDashPattern([], 0);

                pdf.setFont("helvetica", "bold");
                pdf.setFontSize(11);
                pdf.setTextColor(100, 100, 100);
                pdf.text("//NetSnꝍp PisoWifi VOUCHER", x + 5, y + 7);

                pdf.setFont("helvetica", "bold");
                pdf.setFontSize(26);
                pdf.setTextColor(20, 20, 20);
                pdf.text(voucher.code, x + voucherWidth / 2, y + 19, {
                    align: "center",
                });

                pdf.setFont("helvetica", "bold");
                pdf.setFontSize(22);
                pdf.setTextColor(20, 20, 20);
                pdf.text(
                    `PHP ${Number(voucher.price).toFixed(2)}`,
                    x + voucherWidth / 2,
                    y + 32,
                    { align: "center" }
                );

                pdf.setFont("helvetica", "normal");
                pdf.setFontSize(10);
                pdf.setTextColor(70, 70, 70);
                pdf.text(
                    `Time: ${formatSeconds(voucher.time, 0)}`,
                    x + voucherWidth / 2,
                    y + 42,
                    { align: "center" }
                );
            });

            const now = new Date();
            const date = [
                now.getFullYear(),
                String(now.getMonth() + 1).padStart(2, "0"),
                String(now.getDate()).padStart(2, "0"),
            ].join("-");

            pdf.save(`LibreFi-Vouchers-${date}-${selected.length}.pdf`);

            const encryptedPayload = await encrypt(
                getAdminKey(),
                JSON.stringify({ codes: selected.map((voucher) => voucher.code) })
            );

            if (!encryptedPayload) {
                throw new Error("PDF was created, but the download count could not be updated.");
            }

            const response = await fetch("/api/admin/vouchers/pdf-download", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({ payload: encryptedPayload }),
            });

            const result = await response.json();

            if (!response.ok || !result.ok) {
                throw new Error(
                    result.error || "PDF was created, but the download count could not be updated."
                );
            }

            const updatedCounts = new Map<string, number>(
                (result.updated || []).map((item: { code: string; count: number }) => [
                    item.code,
                    item.count,
                ])
            );

            setSelectedVouchers((current) =>
                current.filter((voucher) => !updatedCounts.has(voucher.code))
            );

            await getVouchers();

            toast.success("PDF generated", {
                description: `${selected.length} unused vouchers included in the PDF.`,
                position: "top-center",
            });
        } catch (error) {
            toast.error("PDF download count update failed", {
                description:
                    error instanceof Error
                        ? error.message
                        : "The PDF could not be processed.",
                position: "top-center",
            });

            await getVouchers();
        } finally {
            setDownloading(false);
        }
    }

    const unusedOnPage = (vouchers || []).filter((voucher) => !voucher.used);
    const allUnusedSelected =
        unusedOnPage.length > 0 &&
        unusedOnPage.every((voucher) =>
            selectedVouchers.some((item) => item.code === voucher.code)
        );

    return (
        <div className="h-auto w-full pb-20">
            <div className="w-full flex flex-col md:flex-row">
                <div className="w-full md:w-1/2 border-b-4 md:border-r-4 md:border-b-0 border-foreground/3 p-4">
                    <div className="flex flex-wrap items-center justify-between gap-2 mb-3">
                        <div className="flex items-center gap-2">
                            <label className="flex items-center gap-2 text-xs font-medium">
                                <input
                                    type="checkbox"
                                    className="h-4 w-4 accent-emerald-600"
                                    checked={allUnusedSelected}
                                    disabled={unusedOnPage.length === 0}
                                    onChange={(event) =>
                                        selectAllUnused(event.target.checked)
                                    }
                                />
                                Select all unused on this page
                            </label>
                        </div>

                        <div className="flex flex-wrap items-center gap-2">
                            <Button
                                size="sm"
                                onClick={downloadSelectedPDF}
                                disabled={selectedVouchers.length === 0 || downloading}
                            >
                                <Download size={15} />
                                {downloading
                                    ? "Preparing PDF..."
                                    : `Download Selected (${selectedVouchers.length})`}
                            </Button>

                            <CreateEditVoucher
                                getVouchers={getVouchers}
                                btn={
                                    <Button
                                        size="xs"
                                        variant="secondary"
                                        className="text-emerald-500"
                                    >
                                        <Plus /> New
                                    </Button>
                                }
                            />
                        </div>
                    </div>

                    <Table>
                        {vouchers?.length === 0 ? (
                            <TableCaption>No vouchers found.</TableCaption>
                        ) : null}
                        <TableHeader>
                            <TableRow>
                                <TableHead className="w-8">Select</TableHead>
                                <TableHead className="text-xs font-bold">Code</TableHead>
                                <TableHead>Price</TableHead>
                                <TableHead>Time</TableHead>
                                <TableHead>Status</TableHead>
                                <TableHead>PDF</TableHead>
                                <TableHead className="text-right">Action</TableHead>
                            </TableRow>
                        </TableHeader>
                        <TableBody>
                            {vouchers && vouchers.map((v) => (
                                <TableRow key={v.code}>
                                    <TableCell>
                                        <input
                                            type="checkbox"
                                            className="h-4 w-4 accent-emerald-600"
                                            checked={selectedVouchers.some(
                                                (item) => item.code === v.code
                                            )}
                                            disabled={v.used}
                                            onChange={(event) =>
                                                toggleVoucher(v, event.target.checked)
                                            }
                                            aria-label={`Select voucher ${v.code}`}
                                        />
                                    </TableCell>
                                    <TableCell
                                        onClick={() =>
                                            setVouchers((current) =>
                                                current?.map((vo) => ({
                                                    ...vo,
                                                    censored:
                                                        v.code === vo.code
                                                            ? !vo.censored
                                                            : vo.censored,
                                                }))
                                            )
                                        }
                                        className="text-xs font-bold cursor-pointer"
                                    >
                                        {v.censored ? censor(v.code) : v.code}
                                    </TableCell>
                                    <TableCell>{v.price}</TableCell>
                                    <TableCell>{formatSeconds(v.time)}</TableCell>
                                    <TableCell>
                                        <span className={v.used ? "text-muted-foreground" : "text-emerald-600"}>
                                            {v.used ? "Used" : "Unused"}
                                        </span>
                                    </TableCell>
                                    <TableCell className="font-semibold">
                                        {toRoman((v.pdf_download_count ?? 0) + 1)}
                                    </TableCell>
                                    <TableCell>
                                        <div className="flex justify-end">
                                            <DeleteVoucher getVouchers={getVouchers} code={v.code} />
                                        </div>
                                    </TableCell>
                                </TableRow>
                            ))}
                        </TableBody>
                    </Table>

                    <div className="w-full flex justify-center">
                        <div className="flex items-center justify-between gap-3 mt-4 px-2">
                            <Button
                                variant="outline"
                                size="sm"
                                disabled={voucherPage <= 1}
                                onClick={() => setVoucherPage((page) => Math.max(1, page - 1))}
                            >
                                Prev
                            </Button>
                            <span className="text-sm text-muted-foreground font-medium">
                                Page {voucherPage} of {voucherTotalPages}
                            </span>
                            <Button
                                variant="outline"
                                size="sm"
                                disabled={voucherPage >= voucherTotalPages}
                                onClick={() =>
                                    setVoucherPage((page) =>
                                        Math.min(voucherTotalPages, page + 1)
                                    )
                                }
                            >
                                Next
                            </Button>
                        </div>
                    </div>
                </div>

                <div className="w-full md:w-1/2 p-4">
                    <div>
                        <Table>
                            {rates?.length === 0 ? (
                                <TableCaption>No rates found.</TableCaption>
                            ) : null}
                            <TableHeader>
                                <TableRow>
                                    <TableHead className="text-xs font-bold">Price</TableHead>
                                    <TableHead>Time</TableHead>
                                    <TableHead className="flex justify-end items-center">
                                        <CreateUpdateRate
                                            getRates={getRates}
                                            btn={
                                                <Button
                                                    size="xs"
                                                    variant="secondary"
                                                    className="text-emerald-500"
                                                >
                                                    <Plus /> New
                                                </Button>
                                            }
                                        />
                                    </TableHead>
                                </TableRow>
                            </TableHeader>
                            <TableBody>
                                {rates && rates.map((r) => (
                                    <TableRow key={r.id}>
                                        <TableCell className="text-xs font-bold">
                                            {r.rate.price}
                                        </TableCell>
                                        <TableCell>{formatSeconds(r.rate.time)}</TableCell>
                                        <TableCell className="flex justify-end gap-1">
                                            <DeleteRate
                                                getRates={getRates}
                                                id={r.id}
                                                price={r.rate.price}
                                            />
                                        </TableCell>
                                    </TableRow>
                                ))}
                            </TableBody>
                        </Table>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default AdminVouchRates;

