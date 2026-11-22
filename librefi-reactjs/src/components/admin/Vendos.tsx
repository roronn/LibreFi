import { useCallback, useEffect, useState } from "react";
import { encrypt, getAdminKey } from "#lib/utils";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { toast } from "../ui/toast";
import { Copy, Loader2, Plus, RefreshCw, RotateCw, ShieldOff } from "lucide-react";

type Vendo = {
    vendo_id: string;
    name: string;
    enabled: boolean;
    online: boolean;
    credential_active: boolean;
    created_at: number;
};

type CredentialNotice = {
    vendoName: string;
    credential: string;
};

export default function AdminVendos() {
    const [vendos, setVendos] = useState<Vendo[]>([]);
    const [name, setName] = useState("");
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [busyId, setBusyId] = useState("");
    const [credentialNotice, setCredentialNotice] = useState<CredentialNotice | null>(null);

    const adminPayload = useCallback(async (value: unknown) => {
        const encrypted = await encrypt(getAdminKey(), JSON.stringify(value));
        if (!encrypted) throw new Error("Could not encrypt the admin request.");
        return encrypted;
    }, []);

    const loadVendos = useCallback(async () => {
        setLoading(true);
        try {
            const payload = await adminPayload({ action: "get" });
            const response = await fetch(
                `/api/admin/vendos?payload=${encodeURIComponent(payload)}`
            );
            const data = await response.json();
            if (!response.ok || !data.ok) {
                throw new Error(data.error || `Request failed (HTTP ${response.status})`);
            }
            setVendos(data.vendos || []);
        } catch (error) {
            toast.error("Could not load vendos", {
                position: "top-center",
                description: error instanceof Error ? error.message : "Network error.",
            });
        } finally {
            setLoading(false);
        }
    }, [adminPayload]);

    useEffect(() => {
        void loadVendos();
    }, [loadVendos]);

    async function createVendo() {
        const trimmedName = name.trim();
        if (!trimmedName) return;
        setSaving(true);
        try {
            const response = await fetch("/api/admin/vendos", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    payload: await adminPayload({ name: trimmedName }),
                }),
            });
            const data = await response.json();
            if (!response.ok || !data.ok || !data.credential) {
                throw new Error(data.error || `Registration failed (HTTP ${response.status})`);
            }
            setCredentialNotice({
                vendoName: data.vendo.name,
                credential: data.credential,
            });
            setName("");
            toast.success("Vendo registered", {
                position: "top-center",
                description: "Save its credential now. It will only be shown once.",
            });
            await loadVendos();
        } catch (error) {
            toast.error("Could not register vendo", {
                position: "top-center",
                description: error instanceof Error ? error.message : "Network error.",
            });
        } finally {
            setSaving(false);
        }
    }

    async function updateVendo(vendo: Vendo, action: "enable" | "disable" | "rotate" | "revoke") {
        if (action === "revoke" && !window.confirm(`Revoke the credential for "${vendo.name}"? The device will no longer authenticate.`)) return;
        if (action === "rotate" && !window.confirm(`Rotate the credential for "${vendo.name}"? Its old credential will stop working.`)) return;

        setBusyId(vendo.vendo_id);
        try {
            const method = action === "enable" || action === "disable" ? "PUT" : "POST";
            const endpoint = action === "rotate"
                ? "/api/admin/vendos/rotate"
                : action === "revoke"
                    ? "/api/admin/vendos/revoke"
                    : "/api/admin/vendos";
            const payload = action === "enable" || action === "disable"
                ? { vendo_id: vendo.vendo_id, enabled: action === "enable" }
                : { vendo_id: vendo.vendo_id };
            const response = await fetch(endpoint, {
                method,
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ payload: await adminPayload(payload) }),
            });
            const data = await response.json();
            if (!response.ok || !data.ok) {
                throw new Error(data.error || `Request failed (HTTP ${response.status})`);
            }

            if (action === "rotate") {
                setCredentialNotice({
                    vendoName: vendo.name,
                    credential: data.credential,
                });
                toast.success("Credential rotated", {
                    position: "top-center",
                    description: "Save the new credential now. The previous one is invalid.",
                });
            } else if (action === "revoke") {
                setCredentialNotice(null);
                toast.success("Credential revoked", { position: "top-center" });
            } else {
                toast.success(action === "enable" ? "Vendo enabled" : "Vendo disabled", {
                    position: "top-center",
                });
            }
            await loadVendos();
        } catch (error) {
            toast.error("Vendo update failed", {
                position: "top-center",
                description: error instanceof Error ? error.message : "Network error.",
            });
        } finally {
            setBusyId("");
        }
    }

    async function copyCredential() {
        if (!credentialNotice) return;
        try {
            await navigator.clipboard.writeText(credentialNotice.credential);
            toast.success("Credential copied", { position: "top-center" });
        } catch {
            toast.error("Clipboard unavailable", {
                position: "top-center",
                description: "Select the credential field and copy it manually.",
            });
        }
    }

    return (
        <div className="mx-auto flex max-w-5xl flex-col gap-5 p-4 pt-0 pb-24">
            <section className="rounded-xl border border-border p-5 flex flex-col gap-4">
                <div>
                    <h2 className="text-lg font-semibold">Register a coin vendo</h2>
                    <p className="text-sm text-foreground/60">
                        Each device receives its own credential. The full credential is displayed only after registration or rotation.
                    </p>
                </div>
                <div className="flex flex-col gap-3 sm:flex-row">
                    <div className="flex-1 space-y-1.5">
                        <Label htmlFor="vendo-name">Vendo name</Label>
                        <Input
                            id="vendo-name"
                            value={name}
                            maxLength={80}
                            placeholder="e.g. Front Gate Vendo"
                            onChange={(event) => setName(event.target.value)}
                        />
                    </div>
                    <div className="flex items-end">
                        <Button onClick={createVendo} disabled={saving || !name.trim()}>
                            {saving ? <Loader2 className="mr-2 animate-spin" size={16} /> : <Plus className="mr-2" size={16} />}
                            Register Vendo
                        </Button>
                    </div>
                </div>
            </section>

            {credentialNotice && (
                <section className="rounded-xl border border-emerald-500/30 bg-emerald-500/5 p-5 flex flex-col gap-3">
                    <div>
                        <h2 className="font-semibold text-emerald-600">Save this credential — {credentialNotice.vendoName}</h2>
                        <p className="text-sm text-foreground/60">
                            This is the only time the full credential will be shown. Store it securely in the device configuration.
                        </p>
                    </div>
                    <div className="flex flex-col gap-2 sm:flex-row">
                        <Input
                            readOnly
                            value={credentialNotice.credential}
                            className="font-mono text-xs"
                            onFocus={(event) => event.currentTarget.select()}
                        />
                        <Button variant="outline" onClick={copyCredential}>
                            <Copy className="mr-2" size={16} /> Copy
                        </Button>
                    </div>
                    <div className="flex justify-end">
                        <Button variant="outline" onClick={() => setCredentialNotice(null)}>Close</Button>
                    </div>
                </section>
            )}

            <section className="rounded-xl border border-border p-5 flex flex-col gap-4">
                <div className="flex items-center justify-between gap-3">
                    <div>
                        <h2 className="text-lg font-semibold">Registered vendos</h2>
                        <p className="text-sm text-foreground/60">Online status and admin enablement are independent.</p>
                    </div>
                    <Button variant="outline" size="icon" onClick={() => void loadVendos()} disabled={loading} aria-label="Refresh vendos">
                        {loading ? <Loader2 className="animate-spin" size={16} /> : <RefreshCw size={16} />}
                    </Button>
                </div>

                {loading ? (
                    <div className="flex justify-center p-8"><Loader2 className="animate-spin" /></div>
                ) : vendos.length === 0 ? (
                    <p className="rounded-lg bg-foreground/5 p-5 text-center text-sm text-foreground/60">
                        No vendos registered yet.
                    </p>
                ) : (
                    <div className="flex flex-col gap-3">
                        {vendos.map((vendo) => (
                            <article key={vendo.vendo_id} className="rounded-lg border border-border p-4 flex flex-col gap-3">
                                <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                                    <div className="min-w-0">
                                        <h3 className="font-semibold break-words">{vendo.name}</h3>
                                        <p className="font-mono text-xs text-foreground/50 break-all">{vendo.vendo_id}</p>
                                        <div className="mt-2 flex flex-wrap gap-2 text-xs">
                                            <span className={vendo.online ? "rounded-full bg-emerald-500/10 px-2 py-1 text-emerald-600" : "rounded-full bg-foreground/5 px-2 py-1 text-foreground/60"}>
                                                {vendo.online ? "Online" : "Offline"}
                                            </span>
                                            <span className={vendo.enabled ? "rounded-full bg-emerald-500/10 px-2 py-1 text-emerald-600" : "rounded-full bg-amber-500/10 px-2 py-1 text-amber-600"}>
                                                {vendo.enabled ? "Enabled" : "Disabled"}
                                            </span>
                                            <span className="rounded-full bg-foreground/5 px-2 py-1 text-foreground/60">
                                                {vendo.credential_active ? "Credential active" : "Credential revoked"}
                                            </span>
                                        </div>
                                    </div>
                                    <p className="text-xs text-foreground/50">
                                        Added {new Date(vendo.created_at * 1000).toLocaleString()}
                                    </p>
                                </div>
                                <div className="flex flex-wrap gap-2 border-t border-border pt-3">
                                    <Button
                                        size="sm"
                                        variant="outline"
                                        disabled={busyId === vendo.vendo_id}
                                        onClick={() => void updateVendo(vendo, vendo.enabled ? "disable" : "enable")}
                                    >
                                        {busyId === vendo.vendo_id && <Loader2 className="mr-2 animate-spin" size={14} />}
                                        {vendo.enabled ? "Disable" : "Enable"}
                                    </Button>
                                    <Button
                                        size="sm"
                                        variant="outline"
                                        disabled={busyId === vendo.vendo_id || !vendo.credential_active}
                                        onClick={() => void updateVendo(vendo, "rotate")}
                                    >
                                        <RotateCw className="mr-1" size={14} /> Rotate credential
                                    </Button>
                                    <Button
                                        size="sm"
                                        variant="destructive"
                                        disabled={busyId === vendo.vendo_id || !vendo.credential_active}
                                        onClick={() => void updateVendo(vendo, "revoke")}
                                    >
                                        <ShieldOff className="mr-1" size={14} /> Revoke
                                    </Button>
                                </div>
                            </article>
                        ))}
                    </div>
                )}
            </section>
        </div>
    );
}
