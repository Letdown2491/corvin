// Lightweight password-strength estimate for the at-rest encryption password —
// the sole protection of the on-disk data. A crude entropy estimate (char-pool
// size × length) with penalties for obvious patterns. Deliberately dependency-free
// (no zxcvbn): this is a nudge, not a gate. The backend still enforces length >= 8.

export interface PasswordStrength {
  // 0 = empty, 1 = weak, 2 = fair, 3 = good, 4 = strong.
  score: 0 | 1 | 2 | 3 | 4
  label: string
}

const COMMON =
  /^(0123456789|1234567890|abcdefghij|qwertyuiop|qwerty|password|passw0rd|letmein|welcome|admin|iloveyou|monkey|dragon)/i

export function passwordStrength(pw: string): PasswordStrength {
  if (!pw) return { score: 0, label: '' }

  let pool = 0
  if (/[a-z]/.test(pw)) pool += 26
  if (/[A-Z]/.test(pw)) pool += 26
  if (/[0-9]/.test(pw)) pool += 10
  if (/[^a-zA-Z0-9]/.test(pw)) pool += 32

  let bits = pw.length * Math.log2(pool || 1)

  // Patterns that gut the real entropy regardless of the formula.
  if (/^(.)\1+$/.test(pw)) bits = Math.min(bits, 10) // all one character
  if (COMMON.test(pw)) bits = Math.min(bits, 12) // common sequence/word prefix

  let score: PasswordStrength['score']
  if (bits < 28) score = 1
  else if (bits < 40) score = 2
  else if (bits < 60) score = 3
  else score = 4

  return { score, label: ['', 'Weak', 'Fair', 'Good', 'Strong'][score] }
}
