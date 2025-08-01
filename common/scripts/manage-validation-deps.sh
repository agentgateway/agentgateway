#!/bin/bash

case "$1" in
    start)
        echo "Starting MCP authentication server..."
        python3 examples/mcp-authentication/auth_server.py &
        ;;
    stop)
        pkill -f "examples/mcp-authentication/auth_server.py" 2>/dev/null || true
        ;;
    *)
        echo "Usage: $0 {start|stop}"
        exit 1
        ;;
esac
