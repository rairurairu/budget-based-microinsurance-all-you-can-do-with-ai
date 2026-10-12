# Embed SDK (M8)

A Web Component a partner drops into its app or webview at the moment of need (a delivery app
before typhoon season, a ticketing app at checkout), in a shadow root so the partner's styles and
the embed's stay apart (ADR-011).

1. The partner's server exchanges its own reference for the customer for a short-lived customer
   token: `POST /partners/me/customer-tokens` with the partner's token and
   `{ "customer_ref": "…" }`. The partner never holds the customer's credentials with the insurer.
2. The page loads `/embed/js/embed-sdk/src/index.js` as a module and renders

```html
<microinsurance-embed api="https://api.insurer.example" partner="delivery_app"
                      token="<customer token>" language="zh-Hant"></microinsurance-embed>
```

A licensed partner binds cover in place: every plan, a quote with one sentence per cover, one tap
(R03, R45, R58). A referral-only partner shows the way to the insurer's own journey (R52). The
`language` attribute sets the customer's language setting, so the AI's sentences match. On binding
the element fires `microinsurance:bound` with `{ policyId, starts }`, for example for the partner's
wallet to collect the premium (`POST /partners/me/premium-payments`).

`/embed/` in demo mode is a partner page doing exactly this with the demo partners.

```bash
pnpm --filter @microinsurance/embed-sdk build   # dist/, served by the API at /embed
pnpm --filter @microinsurance/embed-sdk test
```
