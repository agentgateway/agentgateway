import fs from "node:fs";
import { privateKeyToAccount } from "viem/accounts";
import { x402Client } from "@x402/core/client";
import { ExactEvmScheme } from "@x402/evm/exact/client";
import { wrapFetchWithPayment, x402HTTPClient } from "@x402/fetch";

const key = process.env.EVM_PRIVATE_KEY;
if (!key) {
  console.error("EVM_PRIVATE_KEY is required. Use a dedicated Base agent wallet.");
  process.exit(2);
}

const task = process.argv.slice(2).join(" ").trim();
if (!task) {
  console.error('usage: npm run route -- "task text"');
  process.exit(2);
}

const cards = JSON.parse(
  fs.readFileSync(new URL("./agents.json", import.meta.url), "utf8")
);

if (!Array.isArray(cards) || cards.length < 2) {
  throw new Error("agents.json must contain at least two candidate Agent Cards");
}

function stableCandidate(card, index) {
  const name = card.name || `agent-${index + 1}`;
  const description = card.description || "";
  const skills = Array.isArray(card.skills)
    ? card.skills.map((s) =>
        `${s.name || s.id || "skill"}: ${s.description || ""}`
      ).join("; ")
    : "";

  return `${name} — ${description}${skills ? ` — ${skills}` : ""}`;
}

const candidates = cards.map(stableCandidate);

const signer = privateKeyToAccount(key);
const client = new x402Client();
client.register("eip155:*", new ExactEvmScheme(signer));
const paidFetch = wrapFetchWithPayment(fetch, client);
const httpClient = new x402HTTPClient(client);

const response = await paidFetch("https://api.grip.fyi/v1/compare", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    query: task,
    candidates,
    top_k: candidates.length
  })
});

const raw = await response.text();
let result;
try { result = JSON.parse(raw); }
catch { result = raw; }

let settlement = null;
try {
  settlement = httpClient.getPaymentSettleResponse(response.headers);
} catch {}

if (!response.ok) {
  console.error(JSON.stringify({
    http_status: response.status,
    settlement,
    result
  }, null, 2));
  process.exit(1);
}

const rows =
  result?.results ||
  result?.ranked ||
  result?.candidates ||
  [];

const orderedStrings = rows.map((row) => {
  if (typeof row === "string") return row;
  return row?.candidate ?? row?.text ?? row?.value ?? null;
}).filter(Boolean);

const byCandidate = new Map(
  cards.map((card, index) => [candidates[index], card])
);

const orderedCards = orderedStrings
  .map((candidate) => byCandidate.get(candidate))
  .filter(Boolean);

console.log(JSON.stringify({
  task,
  payer: signer.address,
  settlement,
  ordered_agent_cards: orderedCards,
  raw_arbiter_result: result
}, null, 2));
