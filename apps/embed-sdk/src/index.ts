// The embed (M8): a Web Component a distribution partner drops into its app or webview at the
// moment of need. The partner's server first exchanges its own reference for the customer for a
// short-lived customer token (`POST /partners/me/customer-tokens`), then renders:
//
//   <microinsurance-embed api="https://api.insurer.example" partner="delivery_app"
//                         token="…" language="zh-Hant"></microinsurance-embed>
//
// A licensed partner binds cover in place; a referral-only partner shows the way to the
// insurer's own journey (R52). When a policy is bound the element fires `microinsurance:bound`
// with `{ policyId, starts }`, so the partner's app can, for example, collect the premium in its
// wallet. Shadow DOM keeps the partner's styles and the embed's apart.
import { ApiError, createClient, t } from "../../api-client/src/index.ts";
import { type EmbedModel, STYLE, view } from "./views.ts";

export const TAG = "microinsurance-embed";

export class MicroinsuranceEmbed extends HTMLElement {
  static observedAttributes = ["token", "language"];

  private readonly root: ShadowRoot;
  private model: EmbedModel = {
    copy: {},
    language: "en",
    currency: "", // the market's, from /registry
    plans: [],
    covers: [],
    step: { kind: "loading" },
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.addEventListener("click", (e) => this.onClick(e));
  }

  private get client() {
    return createClient({
      baseUrl: this.getAttribute("api") ?? "",
      token: () => this.getAttribute("token"),
    });
  }

  connectedCallback(): void {
    void this.load();
  }

  attributeChangedCallback(): void {
    if (this.isConnected) {
      void this.load();
    }
  }

  private show(): void {
    this.root.innerHTML = `<style>${STYLE}</style><div class="box">${view(this.model)}</div>`;
  }

  private fail(e: unknown): void {
    const detail = e instanceof ApiError && e.body ? e.body.detail : "";
    const text = detail
      ? t(this.model.copy, "errors.refused", { detail })
      : t(this.model.copy, "errors.generic");
    this.model.step = { kind: "error", text };
    this.show();
  }

  private async load(): Promise<void> {
    const api = this.client;
    const partner = this.getAttribute("partner") ?? "";
    try {
      const registry = await api.get("/registry");
      const language = this.getAttribute("language") ?? registry.default_language;
      const pack = await api.get("/copy/{language}", { path: { language } });
      this.model.copy = pack.text;
      this.model.language = pack.language;
      this.model.currency = registry.currency;
      const channel = await api.get("/channels/{partner_id}", { path: { partner_id: partner } });
      if (!channel.binds) {
        this.model.step = { kind: "refer", url: channel.refer_to ?? "" };
        this.show();
        return;
      }
      // The customer's language setting follows the partner app's, so the AI's sentences match.
      const me = await api.get("/me");
      if (this.getAttribute("language") && me.profile.language !== pack.language) {
        await api.patch("/me/profile", { body: { language: pack.language } });
      }
      this.model.covers = await api.get("/covers");
      const held = await api.get("/me/policy").catch((e: unknown) => {
        if (e instanceof ApiError && e.status === 404) {
          return null;
        }
        throw e;
      });
      if (held) {
        this.model.step = {
          kind: "covered",
          starts: held.policy.period.starts,
          policy: held.policy,
        };
      } else {
        this.model.plans = await api.get("/plans", { query: { channel: partner } });
        this.model.step = { kind: "plans" };
      }
      this.show();
    } catch (e) {
      this.fail(e);
    }
  }

  private async onClick(event: Event): Promise<void> {
    const el = (event.target as HTMLElement).closest<HTMLElement>("button");
    if (!el) {
      return;
    }
    const api = this.client;
    try {
      if (el.dataset.plan) {
        const quote = await api.post("/me/quotes", { body: { plan_id: el.dataset.plan } });
        this.model.step = { kind: "quote", quote };
      } else if (el.dataset.back) {
        this.model.step = { kind: "plans" };
      } else if (el.dataset.accept) {
        const done = await api.post("/me/quotes/{quote_id}/accept", {
          path: { quote_id: el.dataset.accept },
        });
        if (done.policy) {
          this.model.step = {
            kind: "covered",
            starts: done.policy.period.starts,
            policy: done.policy,
          };
          this.dispatchEvent(
            new CustomEvent("microinsurance:bound", {
              detail: { policyId: done.policy.policy_id, starts: done.policy.period.starts },
              bubbles: true,
              composed: true,
            }),
          );
        }
      }
      this.show();
    } catch (e) {
      this.fail(e);
    }
  }
}

/** Registers the element once; a partner page calls it after loading the script. */
export function mount(): void {
  if (!customElements.get(TAG)) {
    customElements.define(TAG, MicroinsuranceEmbed);
  }
}

mount();
