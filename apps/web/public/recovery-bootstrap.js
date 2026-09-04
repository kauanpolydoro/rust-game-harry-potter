(() => {
  const fragment = window.location.hash
  if (!fragment.startsWith('#recovery=')) {
    return
  }

  const candidate = new URLSearchParams(fragment.slice(1)).get('recovery')
  const recoveryToken = candidate !== null && /^[0-9a-f]{64}$/.test(candidate) ? candidate : null

  window.history.replaceState(
    window.history.state,
    '',
    `${window.location.pathname}${window.location.search}`,
  )
  Object.defineProperty(window, '__HOGWARTS_RECOVERY_TOKEN__', {
    configurable: true,
    value: recoveryToken,
  })
})()
