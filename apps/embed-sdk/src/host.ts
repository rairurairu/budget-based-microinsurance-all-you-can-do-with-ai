// The demo partner page: what a distribution partner's own code does before rendering the embed.
// Its server holds the partner's token and exchanges its own reference for the customer for a
// short-lived customer token; the page then sets that token on the element. In demo mode the
// partner's token comes from the demo sessions; a real partner keeps it on its server.
import "./index.ts";

interface Session {
  label: string;
  token: string;
  principal: { kind: string; partner_id?: string };
}

const form = document.getElementById("partner") as HTMLFormElement;
const slot = document.getElementById("slot") as HTMLElement;
const log = document.getElementById("log") as HTMLElement;

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const data = new FormData(form);
  const partner = String(data.get("partner"));
  const customerRef = String(data.get("customer_ref"));
  const language = String(data.get("language"));
  const sessions = (await (await fetch("/demo/sessions")).json()) as Session[];
  const partnerToken = sessions.find(
    (s) => s.principal.kind === "partner" && s.principal.partner_id === partner,
  )?.token;
  // On the partner's server: the token exchange.
  const res = await fetch("/partners/me/customer-tokens", {
    method: "POST",
    headers: { Authorization: `Bearer ${partnerToken}`, "Content-Type": "application/json" },
    body: JSON.stringify({ customer_ref: customerRef }),
  });
  const issued = (await res.json()) as { token: string; customer_id: string; expires_at: string };
  log.textContent = `customer token for ${issued.customer_id}, valid until ${issued.expires_at}`;
  const embed = document.createElement("microinsurance-embed");
  embed.setAttribute("partner", partner);
  embed.setAttribute("language", language);
  embed.setAttribute("token", issued.token);
  embed.addEventListener("microinsurance:bound", (e) => {
    log.textContent = `bound: ${JSON.stringify((e as CustomEvent).detail)}`;
  });
  slot.replaceChildren(embed);
});
