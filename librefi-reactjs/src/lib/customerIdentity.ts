import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha2.js";

// Keep Ed25519's SHA-512 implementation independent of crypto.subtle so it
// can also run on LibreFi's plain-HTTP LAN origin.
ed.hashes.sha512Async = async (message) => sha512(message);

const DATABASE_NAME = "librefi-customer-identity";
const STORE_NAME = "keys";
const KEY_NAME = "customer-ed25519-v1";

type StoredKeyPair = {
    privateSeed: string;
    publicKey: string;
};

type CustomerKeyPair = {
    privateSeed: Uint8Array;
    publicKey: Uint8Array;
};

function toHex(bytes: Uint8Array): string {
    return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
}

function fromHex(value: string): Uint8Array {
    if (value.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(value)) {
        throw new Error("Invalid hexadecimal data");
    }
    const bytes = new Uint8Array(value.length / 2);
    for (let index = 0; index < bytes.length; index++) {
        bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
    }
    return bytes;
}

function openIdentityDatabase(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
        if (!("indexedDB" in window)) {
            reject(new Error("IndexedDB is unavailable"));
            return;
        }

        const request = indexedDB.open(DATABASE_NAME, 1);
        request.onupgradeneeded = () => {
            const database = request.result;
            if (!database.objectStoreNames.contains(STORE_NAME)) {
                database.createObjectStore(STORE_NAME);
            }
        };
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error || new Error("Could not open identity storage"));
        request.onblocked = () => reject(new Error("Identity storage is blocked"));
    });
}

async function readStoredKeyPair(): Promise<CustomerKeyPair | null> {
    const database = await openIdentityDatabase();
    try {
        const stored = await new Promise<StoredKeyPair | undefined>((resolve, reject) => {
            const transaction = database.transaction(STORE_NAME, "readonly");
            const request = transaction.objectStore(STORE_NAME).get(KEY_NAME);
            request.onsuccess = () => resolve(request.result as StoredKeyPair | undefined);
            request.onerror = () => reject(request.error || new Error("Could not read identity key"));
        });

        if (!stored || typeof stored.privateSeed !== "string" || typeof stored.publicKey !== "string") {
            return null;
        }

        return {
            privateSeed: fromHex(stored.privateSeed),
            publicKey: fromHex(stored.publicKey)
        };
    } finally {
        database.close();
    }
}

async function storeKeyPair(pair: CustomerKeyPair): Promise<void> {
    const database = await openIdentityDatabase();
    try {
        await new Promise<void>((resolve, reject) => {
            const transaction = database.transaction(STORE_NAME, "readwrite");
            transaction.objectStore(STORE_NAME).put({
                privateSeed: toHex(pair.privateSeed),
                publicKey: toHex(pair.publicKey)
            } satisfies StoredKeyPair, KEY_NAME);
            transaction.oncomplete = () => resolve();
            transaction.onerror = () => reject(transaction.error || new Error("Could not store identity key"));
            transaction.onabort = () => reject(transaction.error || new Error("Identity key storage was aborted"));
        });
    } finally {
        database.close();
    }
}

async function getOrCreateKeyPair(): Promise<CustomerKeyPair | null> {
    try {
        const stored = await readStoredKeyPair();
        if (stored) return stored;

        const generated = await ed.keygenAsync();
        const pair = {
            privateSeed: generated.secretKey,
            publicKey: generated.publicKey
        };
        await storeKeyPair(pair);
        return pair;
    } catch (error) {
        console.warn("Customer key storage is unavailable; using browser-token authentication.", error);
        return null;
    }
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
    });

    if (!response.ok) {
        throw new Error(`Customer identity request failed (${response.status})`);
    }

    return response.json() as Promise<T>;
}

async function signChallenge(
    pair: CustomerKeyPair,
    purpose: "register" | "authenticate",
    challengeId: string,
    challenge: string
): Promise<string> {
    const message = new TextEncoder().encode(
        `LibreFi/customer-auth/v1\n${purpose}\n${challengeId}\n${challenge}`
    );
    return toHex(await ed.signAsync(message, pair.privateSeed));
}

/** Authenticates an existing browser key and returns a fresh browser token. */
export async function authenticateStoredCustomerKey(): Promise<string | null> {
    const pair = await readStoredKeyPair().catch(() => null);
    if (!pair) return null;

    const publicKey = toHex(pair.publicKey);
    const issued = await postJson<{
        challenge_id: string;
        challenge: string;
    }>("/api/customer/key/challenge", {
        purpose: "authenticate",
        public_key: publicKey
    });

    const signature = await signChallenge(
        pair, "authenticate", issued.challenge_id, issued.challenge
    );

    const result = await postJson<{ browser_token?: string }>(
        "/api/customer/key/authenticate",
        {
            purpose: "authenticate",
            challenge_id: issued.challenge_id,
            public_key: publicKey,
            signature
        }
    );

    return typeof result.browser_token === "string" &&
        /^[0-9a-f]{64}$/i.test(result.browser_token)
        ? result.browser_token
        : null;
}

/** Registers the browser's key against the session represented by its token. */
export async function registerCustomerKey(browserToken: string): Promise<void> {
    if (!/^[0-9a-f]{64}$/i.test(browserToken)) return;

    const pair = await getOrCreateKeyPair();
    if (!pair) return;

    const publicKey = toHex(pair.publicKey);
    const issued = await postJson<{
        challenge_id: string;
        challenge: string;
    }>("/api/customer/key/challenge", {
        purpose: "register",
        public_key: publicKey,
        browser_token: browserToken
    });

    const signature = await signChallenge(
        pair, "register", issued.challenge_id, issued.challenge
    );

    await postJson("/api/customer/key/register", {
        purpose: "register",
        challenge_id: issued.challenge_id,
        public_key: publicKey,
        signature,
        browser_token: browserToken
    });
}
