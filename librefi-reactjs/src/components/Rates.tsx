import { useCallback, useRef, useState, type ReactElement } from "react";
import {
    Drawer,
    DrawerContent,
    DrawerTrigger,
} from "../components/ui/drawer";
import {
    Table,
    TableBody,
    TableCaption,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "../components/ui/table";
import type { RateType } from "../types/rates";
import { formatSeconds } from "#lib/utils";

function Rates({ children }: { children: ReactElement }) {
    const [rates, setRates] = useState<RateType[] | null>(null);
    const [open, setOpen] = useState(false);
    const [loading, setLoading] = useState(false);
    const loaded = useRef(false);
    const requestInFlight = useRef(false);

    const getRates = useCallback(async () => {
        if (loaded.current || requestInFlight.current) return;
        requestInFlight.current = true;
        setLoading(true);
        try {
            const res = await fetch("/api/rates");
            if (!res.ok) return;
            const json = await res.json();
            setRates(Array.isArray(json.rows) ? json.rows : []);
            loaded.current = true;
        } catch {
            // Leave the list eligible for retry the next time the drawer opens.
        } finally {
            requestInFlight.current = false;
            setLoading(false);
        }
    }, []);

    const handleOpenChange = useCallback((nextOpen: boolean) => {
        setOpen(nextOpen);
        if (nextOpen) void getRates();
    }, [getRates]);

    return (
        <Drawer open={open} onOpenChange={handleOpenChange}>
            <DrawerTrigger nativeButton={false} render={children}>
                Rates
            </DrawerTrigger>
            <DrawerContent className="flex items-center w-full p-6">
                <div className="max-w-md min-w-xs w-full">
                    <Table>
                        <TableCaption>{loading ? "Loading rates…" : "A list of rates."}</TableCaption>
                        <TableHeader>
                            <TableRow>
                                <TableHead className="text-foreground/20">ID</TableHead>
                                <TableHead>Price</TableHead>
                                <TableHead className="text-right">Time</TableHead>
                            </TableRow>
                        </TableHeader>
                        <TableBody>
                            {rates?.map(rate => (
                                <TableRow key={rate.id}>
                                    <TableCell className="font-medium text-foreground/20">{rate.id}</TableCell>
                                    <TableCell>{rate.rate.price}</TableCell>
                                    <TableCell className="text-right">{formatSeconds(rate.rate.time)}</TableCell>
                                </TableRow>
                            ))}
                        </TableBody>
                    </Table>
                </div>
            </DrawerContent>
        </Drawer>
    );
}

export default Rates;
