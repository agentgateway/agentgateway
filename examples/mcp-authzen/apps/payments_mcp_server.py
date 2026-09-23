#!/usr/bin/env python3
"""Mock payments MCP server for the AuthZEN example.

Exposes 5 tools with canned in-memory data. Authorization is entirely
agentgateway's job (via Keycloak AuthZEN) - this server trusts every call
"""

from mcp.server.mcpserver import MCPServer

mcp = MCPServer("payments")

_ACCOUNTS = {
    "acct-checking": {"id": "acct-checking", "name": "Checking", "balance": 1250.00},
    "acct-savings": {"id": "acct-savings", "name": "Savings", "balance": 8400.50},
}

_payment_seq = 0
_PAYMENTS: dict[str, dict] = {}


@mcp.tool()
def list_accounts() -> list[dict]:
    """List all accounts."""
    return list(_ACCOUNTS.values())


@mcp.tool()
def get_account(account_id: str) -> dict:
    """Get details for one account."""
    account = _ACCOUNTS.get(account_id)
    if account is None:
        return {"error": f"unknown account {account_id}"}
    return account


@mcp.tool()
def create_payment(account_id: str, amount: float, recipient: str) -> dict:
    """Create a new payment in 'pending' status."""
    global _payment_seq
    if account_id not in _ACCOUNTS:
        return {"error": f"unknown account {account_id}"}
    _payment_seq += 1
    payment_id = f"pay-{_payment_seq}"
    _PAYMENTS[payment_id] = {
        "id": payment_id,
        "account_id": account_id,
        "amount": amount,
        "recipient": recipient,
        "status": "pending",
    }
    return _PAYMENTS[payment_id]


@mcp.tool()
def approve_payment(payment_id: str) -> dict:
    """Approve a pending payment."""
    payment = _PAYMENTS.get(payment_id)
    if payment is None:
        return {"error": f"unknown payment {payment_id}"}
    payment["status"] = "approved"
    return payment


@mcp.tool()
def cancel_payment(payment_id: str) -> dict:
    """Cancel a pending payment."""
    payment = _PAYMENTS.get(payment_id)
    if payment is None:
        return {"error": f"unknown payment {payment_id}"}
    payment["status"] = "cancelled"
    return payment


if __name__ == "__main__":
    mcp.run(transport="streamable-http", host="0.0.0.0", port=3001)
