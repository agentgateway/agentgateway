# Build stage: install once
FROM node:20-alpine AS build
ENV NODE_ENV=production NPM_CONFIG_UPDATE_NOTIFIER=false
RUN npm install -g @modelcontextprotocol/server-everything

# Runtime stage: tiny, fast boot
FROM node:20-alpine
ENV NODE_ENV=production

# Bring over node_modules AND bin shims (whatever they’re named)
COPY --from=build /usr/local/ /usr/local/

EXPOSE 3001

# Try server-everything first; fall back to mcp-server-everything if needed
CMD sh -lc '(command -v server-everything >/dev/null && \
  exec server-everything streamableHttp --port 3001) || \
  exec mcp-server-everything streamableHttp --port 3001'
