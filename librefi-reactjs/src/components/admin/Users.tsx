import { encrypt, formatSeconds, getAdminKey } from "#lib/utils";
import { Table, TableBody, TableCaption, TableCell, TableHead, TableHeader, TableRow } from "../ui/table";
import { Button } from "../ui/button";
import { Trash2, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useLocation } from "wouter";

interface UsersT {
    mac: string;
    session_id: string;
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
    const [deletingId, setDeletingId] = useState<string | null>(null);

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

    async function deleteUser(user: UsersT) {
        if (!window.confirm(`Permanently delete ${user.name.trim() || user.mac}? This revokes the customer's browser/device access and releases any vendo reservation.`)) return;
        setDeletingId(user.session_id);
        try {
            const encryptedPayload = await encrypt(getAdminKey(), JSON.stringify({ session_id: user.session_id }));
            if (!encryptedPayload) throw new Error("Could not encrypt the admin request.");
            const response = await fetch("/api/admin/users", {
                method: "DELETE",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ payload: encryptedPayload }),
            });
            const data = await response.json();
            if (!response.ok || !data.ok) throw new Error(data.error || `Delete failed (HTTP ${response.status})`);
            await getUsers();
        } catch (error) {
            window.alert(error instanceof Error ? error.message : "Could not delete user.");
        } finally {
            setDeletingId(null);
        }
    }

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
                            <TableHead className="text-right">Actions</TableHead>
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
                                <TableCell className="text-right">
                                    <Button size="sm" variant="destructive" disabled={deletingId === user.session_id} onClick={() => void deleteUser(user)}>
                                        {deletingId === user.session_id ? <Loader2 className="mr-1 animate-spin" size={14} /> : <Trash2 className="mr-1" size={14} />}
                                        Delete
                                    </Button>
                                </TableCell>
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
