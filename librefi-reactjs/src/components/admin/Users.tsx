import { encrypt, formatSeconds, getAdminKey } from "#lib/utils";
import { Table, TableBody, TableCaption, TableCell, TableHead, TableHeader, TableRow } from "../ui/table";
import { Button } from "../ui/button";
import { useEffect, useState } from "react";
import { useLocation } from "wouter";

interface UsersT {
    mac: string;
    name: string;
    ip: string;
    paused: boolean;
    remaining_seconds: number;
}

function AdminUsers() {
    const [_, setLocation] = useLocation();
    const [users, setUsers] = useState<UsersT[] | null>();
    const [page, setPage] = useState(1);
    const [totalPages, setTotalPages] = useState(1);

    async function getUsers() {
        const encryptedPayload = await encrypt(getAdminKey(), JSON.stringify({ id: 1, page }));
        const res = await fetch("/api/admin/users?payload=" + encodeURIComponent(encryptedPayload || ""));
        if (res.ok) {
            const json = await res.json();
            setUsers(json.users);
            setTotalPages(json.total_pages);
        } else {
            setLocation("/");
        }
    }

    useEffect(() => { void getUsers(); }, [page]);

    return (
        <div>
            <div className="px-2">
                <Table>
                    {users?.length === 0 ? <TableCaption>No users found.</TableCaption> : null}
                    <TableHeader>
                        <TableRow>
                            <TableHead className="text-xs font-bold">MAC</TableHead>
                            <TableHead>IP</TableHead>
                            <TableHead>Name</TableHead>
                            <TableHead>Paused</TableHead>
                            <TableHead className="text-right">Time Left</TableHead>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {users?.map((user) => (
                            <TableRow key={user.mac}>
                                <TableCell className="text-xs font-bold">{user.mac}</TableCell>
                                <TableCell className="text-sm">{user.ip}</TableCell>
                                <TableCell className="text-sm">{user.name.trim() === "" ? "_" : user.name}</TableCell>
                                <TableCell className="text-sm">{String(user.paused)}</TableCell>
                                <TableCell className="text-sm text-right tabular-nums">{formatSeconds(user.remaining_seconds)}</TableCell>
                            </TableRow>
                        ))}
                    </TableBody>
                </Table>
                <div className="w-full flex justify-center">
                    <div className="flex items-center justify-between mt-4 px-2">
                        <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((p) => Math.max(1, p - 1))}>Prev</Button>
                        <span className="text-sm text-muted-foreground font-medium">Page {page} of {totalPages}</span>
                        <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((p) => Math.min(totalPages, p + 1))}>Next</Button>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default AdminUsers;
