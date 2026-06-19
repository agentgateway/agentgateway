export function gatewayOrigin(port: number) {
  const protocol = window.location.protocol || "http:";
  const hostname = bracketIpv6(window.location.hostname || "localhost");
  return `${protocol}//${hostname}:${port}`;
}

export function gatewayEndpoint(port: number, path = "") {
  return `${gatewayOrigin(port)}${path}`;
}

function bracketIpv6(hostname: string) {
  return hostname.includes(":") && !hostname.startsWith("[")
    ? `[${hostname}]`
    : hostname;
}
