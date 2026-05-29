/**
 * Returns the effective hostname for a listener, falling back to "localhost"
 * when the hostname is unset or a wildcard.
 */
export const getListenerHostname = (hostname: string | null | undefined): string =>
  !hostname || hostname === "*" ? "localhost" : hostname;
