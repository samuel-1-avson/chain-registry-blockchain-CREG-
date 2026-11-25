if (typeof window !== 'undefined' && window.trustedTypes?.createPolicy) {
  try {
    window.trustedTypes.createPolicy('creg-default', {
      createScriptURL(url) {
        const parsed = new URL(url, window.location.href)
        if (parsed.origin !== window.location.origin) {
          throw new TypeError(`Blocked cross-origin script URL: ${parsed.origin}`)
        }
        return parsed.toString()
      },
      createHTML() {
        throw new TypeError('HTML injection is forbidden under creg-default policy')
      },
      createScript() {
        throw new TypeError('Dynamic script evaluation is forbidden under creg-default policy')
      },
    })
  } catch {
    // Policy already registered in HMR/test contexts.
  }
}
