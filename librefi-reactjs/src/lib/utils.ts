import { gcm } from "@noble/ciphers/aes.js";
import { randomBytes } from "@noble/ciphers/utils.js";

import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatSeconds(totalSeconds: number, maxUnits = 2) {
  if (!totalSeconds || totalSeconds <= 0) return '0 seconds';

  const timeUnits = [
    { label: 'year', seconds: 31536000 },
    { label: 'month', seconds: 2592000 },
    { label: 'day', seconds: 86400 },
    { label: 'hour', seconds: 3600 },
    { label: 'minute', seconds: 60 },
    { label: 'second', seconds: 1 }
  ];

  const parts = [];
  let remainingSeconds = totalSeconds;

  for (const unit of timeUnits) {
    if (remainingSeconds >= unit.seconds) {
      const count = Math.floor(remainingSeconds / unit.seconds);
      remainingSeconds %= unit.seconds; // pass the remainder to the next loop

      parts.push(`${count} ${unit.label}${count !== 1 ? 's' : ''}`);
    }
  }

  const displayParts = maxUnits ? parts.slice(0, maxUnits) : parts;

  if (displayParts.length === 0) return '0 seconds';
  if (displayParts.length === 1) return displayParts[0];
  if (displayParts.length === 2) return `${displayParts[0]} and ${displayParts[1]}`;

  const lastPart = displayParts.pop();
  return `${displayParts.join(', ')} and ${lastPart}`;
}


export async function encrypt(keyStr: string, payloadStr: string) {
  try {
    JSON.parse(payloadStr);

    // Match the Rust backend: UTF-8 key bytes, zero-padded or truncated to 32 bytes.
    const enc = new TextEncoder();
    const rawKey = enc.encode(keyStr);
    const keyBytes = new Uint8Array(32);
    keyBytes.set(rawKey.slice(0, 32));

    // Use a vetted pure-JavaScript AES-GCM implementation. Unlike crypto.subtle,
    // this works when LibreFi is opened over plain HTTP on a private LAN IP.
    // crypto.getRandomValues (used by randomBytes) remains available in this context.
    const nonce = randomBytes(12);
    const ciphertext = gcm(keyBytes, nonce).encrypt(enc.encode(payloadStr));

    // Backend format: Base64([12-byte nonce] + [AES-GCM ciphertext + tag]).
    const combined = new Uint8Array(12 + ciphertext.byteLength);
    combined.set(nonce, 0);
    combined.set(ciphertext, 12);

    return btoa(String.fromCharCode(...combined));
  } catch (e) {
    console.error("Admin payload encryption failed:", e);
    return null;
  }
}

export function getAdminKey() {
  return window.localStorage.getItem("adminkey") || "";
}