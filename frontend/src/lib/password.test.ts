import { describe, it, expect } from 'vitest'
import { passwordStrength } from './password'

describe('passwordStrength', () => {
  it('returns empty score for an empty string', () => {
    expect(passwordStrength('')).toEqual({ score: 0, label: '' })
  })

  it('rates repeated and common passwords weak', () => {
    expect(passwordStrength('aaaaaaaa').score).toBe(1)
    expect(passwordStrength('password').score).toBe(1)
    expect(passwordStrength('12345678').score).toBe(1)
  })

  it('rates a short mixed password as not strong', () => {
    expect(passwordStrength('aB3!').score).toBeLessThanOrEqual(2)
  })

  it('rates a long high-variety passphrase strong', () => {
    expect(passwordStrength('Tr0ub4dour&3xpl0re-Mountain').score).toBe(4)
  })

  it('increases monotonically as a password grows', () => {
    const short = passwordStrength('Ab1!xy').score
    const long = passwordStrength('Ab1!xyQ9wz#Lm7$').score
    expect(long).toBeGreaterThanOrEqual(short)
  })
})
