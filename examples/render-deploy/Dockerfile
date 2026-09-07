# Wrapper around the official image so Render can persist /config and
# protect /ui/ with HTTP basic auth.
#
# The published image has no shell (ENTRYPOINT=/app/agentgateway). Empty
# /config auto-gen serves /ui/ with no auth. This image writes
# /config/.htpasswd from UI_USER / UI_PASSWORD on every start, seeds
# config.yaml on first boot, then execs the gateway.
#
# Pin: https://agentgateway.dev/docs/standalone/latest/setup/install/docker/
#
# The published image USER is 65532. Render disks are typically
# root-owned, so first boot cannot write /config unless we run as root.

ARG AGENTGATEWAY_VERSION=v1.5.0

FROM cr.agentgateway.dev/agentgateway:${AGENTGATEWAY_VERSION} AS agw

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=agw /app/agentgateway /app/agentgateway
COPY entrypoint.sh /entrypoint
RUN chmod +x /entrypoint \
 && /app/agentgateway --version
USER 0
EXPOSE 4000
ENTRYPOINT ["/entrypoint"]
