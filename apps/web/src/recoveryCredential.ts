const recoveryTokenPattern = /^[0-9a-f]{64}$/

export function takeRecoveryToken(): string | null {
  const token = window.__HOGWARTS_RECOVERY_TOKEN__
  delete window.__HOGWARTS_RECOVERY_TOKEN__
  return typeof token === 'string' && recoveryTokenPattern.test(token) ? token : null
}
