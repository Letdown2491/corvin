// Passphrase-encrypted backup envelope. AES-256-GCM for the payload; the key is
// derived from the passphrase with Argon2id (memory-hard) for new backups, and
// PBKDF2-SHA256 for ones written by older Corvin versions. Everything happens
// client-side so the server never sees the plaintext or the passphrase.
//
// On-disk format (JSON), Argon2id (current):
//   {
//     "corvin_backup_v1_encrypted": true,
//     "kdf": "argon2id",
//     "mem_kib": 19456, "time_cost": 3, "parallelism": 1,
//     "salt": "<base64>", "iv": "<base64>",
//     "ciphertext": "<base64>"   // AES-GCM output (includes tag)
//   }
//
// Legacy format (still decryptable):
//   { ..., "kdf": "pbkdf2-sha256", "iterations": 200000, ... }

import { argon2idAsync } from '@noble/hashes/argon2.js'

const KEY_LEN = 256
const SALT_LEN = 16
const IV_LEN = 12

// Argon2id cost parameters for new backups. Memory-hard; tuned for an offline,
// one-shot backup KDF that runs in pure JS. OWASP's argon2id floor is
// m=19 MiB / t=2 / p=1; we use t=3 for a little extra margin.
const ARGON2_MEM_KIB = 19_456
const ARGON2_TIME = 3
const ARGON2_PARALLELISM = 1
const ARGON2_DKLEN = 32

interface BaseEnvelope {
  corvin_backup_v1_encrypted: true
  salt: string
  iv: string
  ciphertext: string
}

interface Pbkdf2Envelope extends BaseEnvelope {
  kdf: 'pbkdf2-sha256'
  iterations: number
}

interface Argon2Envelope extends BaseEnvelope {
  kdf: 'argon2id'
  mem_kib: number
  time_cost: number
  parallelism: number
}

export type EncryptedEnvelope = Pbkdf2Envelope | Argon2Envelope

export function isEncryptedEnvelope(obj: unknown): obj is EncryptedEnvelope {
  return !!obj && typeof obj === 'object' && (obj as Record<string, unknown>).corvin_backup_v1_encrypted === true
}

function toBase64(bytes: Uint8Array): string {
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}

function fromBase64(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

// Derive an AES-GCM key from raw KDF output. The bytes are zeroed after import.
async function importAesKey(keyBytes: Uint8Array): Promise<CryptoKey> {
  try {
    return await crypto.subtle.importKey(
      'raw',
      keyBytes as unknown as BufferSource,
      { name: 'AES-GCM', length: KEY_LEN },
      false,
      ['encrypt', 'decrypt'],
    )
  } finally {
    keyBytes.fill(0)
  }
}

async function deriveKeyArgon2(
  passphrase: string,
  salt: Uint8Array,
  memKib: number,
  timeCost: number,
  parallelism: number,
): Promise<CryptoKey> {
  const raw = await argon2idAsync(passphrase, salt, {
    m: memKib,
    t: timeCost,
    p: parallelism,
    dkLen: ARGON2_DKLEN,
    asyncTick: 20,
  })
  return importAesKey(raw)
}

async function deriveKeyPbkdf2(passphrase: string, salt: Uint8Array, iterations: number): Promise<CryptoKey> {
  const enc = new TextEncoder()
  const material = await crypto.subtle.importKey(
    'raw',
    enc.encode(passphrase),
    'PBKDF2',
    false,
    ['deriveKey'],
  )
  return crypto.subtle.deriveKey(
    {
      name: 'PBKDF2',
      salt: salt as unknown as BufferSource,
      iterations,
      hash: 'SHA-256',
    },
    material,
    { name: 'AES-GCM', length: KEY_LEN },
    false,
    ['encrypt', 'decrypt'],
  )
}

export async function encryptBackup(plaintext: string, passphrase: string): Promise<EncryptedEnvelope> {
  const salt = crypto.getRandomValues(new Uint8Array(SALT_LEN))
  const iv = crypto.getRandomValues(new Uint8Array(IV_LEN))
  const key = await deriveKeyArgon2(passphrase, salt, ARGON2_MEM_KIB, ARGON2_TIME, ARGON2_PARALLELISM)
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv: iv as unknown as BufferSource },
      key,
      new TextEncoder().encode(plaintext),
    ),
  )
  return {
    corvin_backup_v1_encrypted: true,
    kdf: 'argon2id',
    mem_kib: ARGON2_MEM_KIB,
    time_cost: ARGON2_TIME,
    parallelism: ARGON2_PARALLELISM,
    salt: toBase64(salt),
    iv: toBase64(iv),
    ciphertext: toBase64(ciphertext),
  }
}

export async function decryptBackup(env: EncryptedEnvelope, passphrase: string): Promise<string> {
  const salt = fromBase64(env.salt)
  const iv = fromBase64(env.iv)
  const ciphertext = fromBase64(env.ciphertext)

  let key: CryptoKey
  if (env.kdf === 'argon2id') {
    // Bound the cost parameters: reject absurd values that would either be
    // forgeable (too cheap) or a decrypt-time OOM/DoS (too expensive).
    const { mem_kib, time_cost, parallelism } = env
    if (![mem_kib, time_cost, parallelism].every((n) => typeof n === 'number' && Number.isFinite(n))) {
      throw new Error('Invalid Argon2 parameters in backup envelope')
    }
    // Cap memory at 256 MiB (13× our own 19 MiB default) so a hostile/corrupt
    // file can't trigger a multi-hundred-MB pure-JS allocation that hangs or
    // OOMs the tab on a decrypt attempt.
    if (mem_kib < 8 || mem_kib > 262_144 || time_cost < 1 || time_cost > 32 || parallelism < 1 || parallelism > 16) {
      throw new Error('Argon2 parameters in backup envelope are out of range')
    }
    key = await deriveKeyArgon2(passphrase, salt, mem_kib, time_cost, parallelism)
  } else if (env.kdf === 'pbkdf2-sha256') {
    // Sanity-bound the iteration count: refuse trivially-low values (forgery
    // resistance) and absurdly high ones (a DoS where decrypting takes forever).
    if (typeof env.iterations !== 'number' || env.iterations < 10_000 || env.iterations > 10_000_000) {
      throw new Error('Invalid iteration count in backup envelope')
    }
    key = await deriveKeyPbkdf2(passphrase, salt, env.iterations)
  } else {
    throw new Error(`Unsupported KDF: ${(env as { kdf: string }).kdf}`)
  }

  try {
    const plaintext = await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: iv as unknown as BufferSource },
      key,
      ciphertext as unknown as BufferSource,
    )
    return new TextDecoder().decode(plaintext)
  } catch {
    // AES-GCM throws an opaque DOMException on auth-tag mismatch.
    throw new Error('Wrong passphrase or file is corrupted')
  }
}
