# Wrapper around the official image so Render can persist /config and
# protect /ui/ with HTTP basic auth.
#
# The published image has no shell (ENTRYPOINT=/app/agentgateway). Empty
# /config auto-gen serves /ui/ with no auth. This image writes
# /config/.htpasswd from UI_USER / UI_PASSWORD on every start, seeds
# config.yaml (UI basicAuth, OpenAI wildcard, virtual keys, GitHub MCP)
# on first boot, then execs the gateway.
#
# Pin: https://agentgateway.dev/docs/standalone/latest/setup/install/docker/
#
# The published image USER is 65532. Render disks are typically root-owned,
# so this image starts as root to claim /config, then the entrypoint chowns
# the disk and drops to 65532 via setpriv before exec'ing the gateway.

ARG AGENTGATEWAY_VERSION=v1.5.0

FROM cr.agentgateway.dev/agentgateway:${AGENTGATEWAY_VERSION} AS agw

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends apache2-utils ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# apache2-utils for `htpasswd -B` (bcrypt); setpriv ships in util-linux.
COPY --from=agw /app/agentgateway /app/agentgateway
COPY entrypoint.sh /entrypoint
RUN chmod +x /entrypoint \
 && /app/agentgateway --version
# Root is only for chowning the mounted disk; the entrypoint drops to 65532.
USER 0
EXPOSE 4000
ENTRYPOINT ["/entrypoint"]
