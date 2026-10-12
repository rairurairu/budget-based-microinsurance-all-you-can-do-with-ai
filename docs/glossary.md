# Glossary: industry terms

Code, config, docs and prompts use the **industry term**. Customer-facing copy may stay plain
(for example "missions" in the app), but never in code or config. The "Was" column lists the
internal terms this repo used before; do not reintroduce them.

## Product and policy

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| plan | 保障計劃 | `Plan`, `PlanId`, `plans:` | tier | Starter / Essential / Plus / Max; the customer picks one and it is never recommended; may carry privileges (R01, R45) |
| upcoming event | 預期事件 | `UpcomingEvent`, `UpcomingEventKind` | plan (future) | A concert, a trip, a new device |
| policy | 保單 | `Policy` | | |
| policy period | 保單期 | `PolicyPeriod`, `policy_period:` | cycle | One month by default |
| grace period, lapse, reinstatement | 寬限期、斷保、復效 | `grace_period_days`, `reinstatement_days` | | R53 |
| licensed insurance agency, broker | 持牌保險代理機構、經紀 | `distribution_partners` | | Needed to bind (R52) |
| record retention | 紀錄保留 | `record_retention_years` | | R54 |
| automatic renewal | 自動續保 | `auto_renewal:` | carry over | Unchanged premium allocation renews at the new rates (R34) |
| renewal notice | 續保通知 | `renewal_notice_days` | repricing notice | R35 |
| offer, acceptance | 要約、承諾 | `Offer`, `OfferKind`, `consent::accept` | proposal (consent) | Terms put to the customer; a tap accepts (R03) |
| quotation (quote) | 報價 | `Quote`, `POST /me/quotes` | | An offer kept with everything it showed: the suggestion and its factors, the sentences, the price (R49); valid on the day it was made (R58) |
| new business, binding, inception | 新造業務、承保生效、生效日 | `OfferKind::NewPolicy`, `consent::bind` | subscribe | The tap that makes a policy; cover incepts at 00:00 the next day (R58) |
| effective date (of a rate) | 費率生效日 | `rating::horizon_cost` | | A period takes the rates in force on its first day (R31, ADR-011) |
| endorsement | 批單 | `Endorsement` | scheduled change | An accepted change, effective next policy period (R04) |
| schedule | 保障表 | `Schedule` | cover mix | The covers and sums insured for one policy period |
| cover | 保障 | `CoverSpec`, `CoverId` | | HK and UK usage; one cover per catalogue file |
| product limit | 產品限額 | `product_limit` | | The most sum insured (or benefit amount per event) one customer can hold in a cover (R34) |
| key features | 產品摘要 | `key_features` | customer outcome | One plain sentence per cover |
| first-loss basis, average clause | 第一損失基礎、比例分攤條款 | | | First loss: paid in full up to the sum insured; no average clause (R48) |
| sum insured | 保額 | `Allocation::sum_insured` | cover amount, benefit | For parametric covers, the benefit amount: per day for a daily benefit, otherwise per loss occurrence |
| benefit amount | 賠償金額 | | payout per event | Fixed amount a parametric cover pays per day or per loss occurrence, by its payout basis |
| premium allocation | 保費分配 | `Allocation` | | Share of the plan price per cover |
| allocation step | 分配單位 | `allocation_step` | rebalance step | HK$5 default |
| proposal form | 投保書 | `ProposalForm`, `ProposalQuestion` | questionnaire | What `ai::understand` collects |
| promotional discount | 推廣優惠 | `promotions:` | | The only way a plan price goes down (R44) |
| premium rebate | 保費回贈 | `ScoreMode::PremiumRebate` | lower price | Credited after the policy period; the plan price is unchanged (R11) |
| bonus sum insured | 額外保額 | `ScoreMode::BonusSumInsured` | add cover | More cover at the same price (R11) |
| waiting period, eligibility, loss ratio, premium | 等候期、資格、賠付率、保費 | | | |

## Rating and underwriting

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| rate | 費率 | `Rate`, `CoverRating::rate` | pricing table | Premium per unit of sum insured, in parts per million (ADR-004) |
| rating basis, expected claim cost | 費率基礎、預期賠款成本 | `RatingBasis`, `expected_cost` | | Market data per policy period from the economic models and filed rates; never from AI |
| rating factor, relativity | 費率因子、相對系數 | `config/rating_factors.yaml` | | `industry` or `novel` (R43) |
| rating horizon | 費率評估期 | `rating_horizon_periods` | look-ahead | R32 |
| portfolio average | 組合平均 | | pool average | |
| undeclared group | 未申報組別 | `undeclared_group_average` | | Customers who withheld a factor are rated on their own average (R33) |
| continuous cover | 持續受保 | `continuous_cover_periods` | | R32 |
| line of business, class of business | 業務線、業務類別 | `config/lines_of_business.yaml`, `class_of_business:` | domain | Depth 1 = line, depth 2 = class, deeper = sub-class |
| points pool | 積分池 | `points_pool_depth` | isolation group | Points and rewards never cross it (R36) |
| re-underwriting | 重新核保 | | | Every policy period is a renewal (R39) |
| risk class: preferred, standard, substandard | 優良、標準、次標準 | `underwriting.risk_classes` | | R39 |
| claims experience, credibility | 索償經驗、可信度 | `claims_experience`, `credibility_full_periods` | | Evidence of exposure, not fault; parametric payouts count when above peers (R39) |
| notice of exclusion (change in terms at renewal) | 除外通知（續保條款變更） | `exclusion_notice_periods`, `ClassStatus::ExclusionNotice` | notice of non-renewal, suspension notice | The policy renews; one class is excluded after the notice, if that period's re-underwriting still supports it (R41). Non-renewal happens only on the customer's cancellation |
| re-entry waiting period | 復效等候期 | `reentry_waiting_period_days` | | After a postponement ends (R47) |
| postponed | 延期承保 | `postponed`, `postponement_periods` | suspended, 凍結 | Not accepted for a set period, then reassessed (R41, ADR-003) |
| exposure | 風險暴露 | `exposure:`, `Exposure` | | What a customer must have for a cover to protect them; the optimiser places premium only there (R42) |
| peer-relative experience | 同儕相對經驗 | `claims_experience_threshold` | | Own claim frequency against peers with the same rated exposure; common shocks cancel (R39) |
| experience link | 經驗關聯 | `experience_links` | | The only way a claim in one class moves another (R39) |

## Claims and payments

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| parametric cover, trigger, trigger event | 參數式保障、觸發條件、觸發事件 | `CoverType::Parametric`, `TriggerEvent` | | One event type in force in one zone from first report to end; a typhoon raised and lowered is one event (ADR-006) |
| source report | 數據源報告 | `SourceReport` | feed event | One report in the feed's own codes, before classification (R55) |
| trigger event type, stage | 觸發事件類型、級別 | `config/event_types.yaml`, `EventType`, `Occurrence` | trigger code | What a cover pays on, independent of the feed; stages ordered by severity (R55) |
| data source (index source) | 數據來源（指數來源） | `config/triggers/*.yaml`, `DataSource` | | Maps its own codes to event types and stages (R55) |
| pricing basis | 定價基礎 | `pricing_basis` | | The source's historical record or the insurer's filed rate (R55) |
| payout | 賠付 | `payouts` | | Standard for parametric covers |
| peril | 風險類別（險別） | `accumulation.perils`, `Peril` | hazard group | Event types one physical event produces together (a typhoon's signals and its rain), as the reinsurance programme defines a loss (R51, ADR-006) |
| loss occurrence, aggregation window | 單一事故、事故彙總期 | `LossOccurrence`, `aggregation_hours` | event (for limits), hours clause | A peril's trigger events in one zone, each starting within `aggregation_hours` of the last ending: one event limit, one payout per stage (R27, R51) |
| resumption time, minimum disrupted time | 復工時間、最少受影響工時 | `resumption_minutes`, `min_disrupted_minutes` | grace hours | How long after a trigger ends work is still disrupted, and how much declared work must be lost for a day to count (R08, ADR-006) |
| event limit, pro-rata reduction | 每次事件上限、按比例扣減 | `Peril::event_limit` | | One loss occurrence's payouts together (R51) |
| accumulation control | 累積控制 | `zone_accumulation_limit` | | Benefit in force per peril and zone (R51) |
| two-stage payout | 兩階段賠付 | `PayoutStage`, `OccurrencePayouts` | | Guaranteed share at once, balance after the loss occurrence closes (R51, ADR-006) |
| claim payment | 理賠款 | | payout (indemnity) | Indemnity claims |
| straight-through processing (STP) | 自動理賠 | `StpOutcome` | automated outcome | Approve or refer only (R13) |
| referral | 轉介人工 | `StpOutcome::Referred` | | |
| claim decline | 拒賠 | `claims::person_decision` | | People only (R13) |
| settlement in kind | 實物賠償 | `Settlement::InKind`, `settlement_in_kind` | alternative fulfilment | Repair, replacement, credit (R28) |
| claims audit | 理賠抽查 | `guardrails::audit` | | R14 |
| review request | 覆核申請 | `guardrails::appeals`, `claims::appeal` | appeal | Decided by a person other than the one who decided (four eyes); not the regulator's complaints route (R15) |
| guarantee of payment | 擔保付款 | `auto_guarantee_cap` | | Automatic only up to the cap (R46) |
| emergency assistance, first response | 緊急援助、即時回應 | `human_care.first_response`, `urgent_response_seconds` | | Automatic, within seconds, before any person (R46) |
| vulnerable customer | 弱勢客戶 | `guardrails.human_care` | | Gets a person when needed (R46) |
| first notification of loss (FNOL) | 首次損失通知 | `ClaimNotice`, `claims::notify` | claim form | Every notification is registered; triaged at once (R13, R46) |
| straight-through processing authority, STP limit | 自動理賠權限、自動理賠上限 | `claims.stp_limit`, `stp_min_confidence`, `estimate_tolerance` | | When a claim is approved without a person (R57, ADR-010) |
| claim assessor, loss estimate | 理賠評估、損失估算 | `ai::assess`, `Assessment::estimate` | | Approve or refer; the estimate checks the amount claimed, never sets the amount paid (R13, R48) |
| fraud indicator (red flag), anomaly score | 欺詐指標、異常分數 | `guardrails::anomaly`, `claims.anomaly` | | Named indicators with configured weights; at the risk review threshold a person looks (R14) |
| decline reason | 拒賠理由 | `DeclineReason` | | A closed list tied to the wording; only a person declines (R13) |
| audit finding, overturn rate | 稽核結果、推翻率 | `AuditFinding`, `audit::overturn_rates` | | The random sample's overturn rate estimates the automated path's error (R14) |
| adverse impact ratio (four-fifths rule) | 不利影響比率（五分之四規則） | `guardrails.fairness`, `fairness::parity_reports` | parity gap | A group below `min_parity_ratio` of the best group's approval rate is flagged (R19) |
| work queue, service level | 工作隊列、服務時限 | `guardrails::queue`, `referral_sla_hours`, `appeal_sla_hours` | | The ops console's one queue: urgent first, then due time |
| decision log | 決策紀錄 | `DecisionRecord` | | Who decided what, on which inputs (hashed), so it can be reviewed and replayed |

## Prevention and rewards

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| loss prevention task | 防損任務 | `PreventionTaskDraft`, `engine::prevention`, `ai::prevention` | mission | "Mission" stays as app copy |
| cost-benefit test | 成本效益測試 | `prevention::admit` | prevention test | R18 |
| verified prevention activity | 已驗證防損活動 | `VerifiedActivity` | verified action | |
| loyalty points | 積分 | `VerifiedActivity::points`, `score::points` | | |
| reward, incentive | 獎勵 | `RewardOffer` | | |
| loss prevention measure (risk improvement) | 防損措施 | `config/prevention_measures.yaml`, `PreventionMeasure` | | What the insurer recognises as a prevention task, with its claims data; the AI picks one (R18, ADR-009) |
| prevention basis | 防損基礎數據 | `PreventionBasis`, `MeasureClaims` | | Each measure's claim probability without and with it and its class's average claim cost, from claims data; never AI |
| repeat window | 重複期 | `repeat_after_days` | | One verified activity per measure per customer in it (R10) |
| pre-payment audit sample, risk review | 付款前抽查、風險覆核 | `guardrails::audit::select`, `Referral::Audit` | | 1 in 20 passes held for a person before credit; anomaly at the threshold always (R14) |
| reward balance | 獎賞結餘 | `rewards::balance`, ledger `rewards_payable` | | Granted rewards less what was spent; cash for all of it is always first on the menu (R16) |
| monthly reward cap | 每月獎賞上限 | `monthly_cap_per_customer`, `rewards::grant` | | Per customer and calendar month (R17) |
| loyalty (participation) reward | 參與獎賞 | `rewards::loyalty`, `EarnedFor::ParticipationStreak` | no-loss reward | For a streak of periods with verified activity; never for not claiming (R26) |
| supplier panel, supplier | 服務供應商網絡、供應商 | `config/suppliers.yaml`, `Supplier` | | Delivers settlement in kind and partner rewards under a supply agreement (R28, R56) |
| tariff | 協議價 | `Tariff` | marginal cost | What the insurer pays a supplier per delivery: fixed, or invoiced up to the amount reserved |
| retail value, fair value | 零售價值、公平價值 | `retail_value`, `payouts::fair_value` | | In kind replaces cash only when worth at least the cash (R28) |
| in-kind order | 實物賠償指示 | `InKindOrder` | | Issued, delivered or expired; unused goes back to the customer (R56) |
| supplier statement | 供應商月結單 | `SupplierStatement` | | One per supplier and month, for deliveries only (R56) |
| reward voided (clawback) | 撤銷獎賞 | `rewards::void`, `Voided` | | Only on a person's finding of fraud; what is left in the balance is taken back |
| distribution partner | 分銷夥伴 | `PartnerId`, `apps/partner-console` | | The app the cover is embedded in; also supplies rewards |
| introducer (referral-only partner) | 轉介人 | `LicenceType::ReferralOnly`, `ChannelInfo::refer_to` | | Refers the customer to the insurer's own journey; never binds (R52) |
| premium collection (by the agent) | 代收保費 | `POST /partners/me/premium-payments`, `postings::premium_received` | | The partner's wallet collects the premium and accounts for it to the insurer (R53) |
| small-cell suppression | 小群組抑制 | `channels.partner_insights_min_cohort`, `insights::partner_results` | MIN_COHORT | A figure from too few customers is withheld from a partner (R21) |

## AI layer

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| AI recommendation | AI 建議 | typed output of `crates/ai`, `Harnessed<T>` | proposal | In insurance a proposal is an application for cover |
| saved answer, fallback | 預設答案 | `Source::Fallback`, each component's `fallback` | | What the customer gets when a model is slow, fails or answers outside the rules (R22) |
| saved reply | 已存模型回覆 | `fixtures/ai_replies`, `replay::ReplayModel` | | A model reply kept for demo mode and offline evaluation |
| language pack | 語言設定檔 | `config/languages/<tag>.yaml`, `language::Languages` | | A language a customer can choose: its check terms and saved answers |
| golden set | 黃金測試集 | `python/evals/golden` | | Cases with expected properties, run before any prompt change ships |
| prevention task proposal | 防損任務建議 | `PreventionTaskProposal` | | The AI's action and evidence; the engine adds the amounts |

## Channels and systems

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| policy administration system (system of record) | 保單行政系統 | `crates/store`, `collections` | | Every record the channels and the workers keep; versioned writes (ADR-011) |
| optimistic locking | 樂觀鎖定 | `Record::version`, `StoreError::Conflict` | | A write names the version it read, so a quote binds once |
| principal, bearer token | 主體、持有者權杖 | `auth::Principal`, `TokenKeys` | | Customer, partner, a named person at the insurer, or a data source |
| token exchange | 權杖交換 | `POST /partners/me/customer-tokens` | | The partner's server swaps its own customer reference for a short-lived customer token (RFC 8693) |
| segregation of duties (four eyes) | 職責分離（四眼原則） | `StaffCaller`, `claims::person_decision` | | The person deciding is the token's; a decline is reviewed by someone else (R15) |
| embedded insurance | 嵌入式保險 | `apps/embed-sdk` (`<microinsurance-embed>`) | | Cover offered inside the partner's own app at the moment of need |
| approved customer wording (copy) | 已批核的客戶措辭 | `config/copy/<tag>.yaml` | | The words the apps show, per offered language, checked at start-up |

## Adoption and operations (ADR-012)

| Industry term | 中文 | In code / config | Was | Note |
|---|---|---|---|---|
| market overlay (country configuration) | 市場設定 | `config/markets/<market>/`, `MARKET`, `ConfigDir` | | A market's own files over `config/`: its insurer profile, sources, stages, wording |
| Common Alerting Protocol (CAP) | 共通示警協議 | `adapter: cap`, `adapters/cap.rs` | | OASIS CAP 1.2 / ITU-T X.1303: the standard format for public warnings |
| work suspension | 停止上班（停班） | `work.suspension` | | The local government's announcement that work stops for a natural disaster; Taiwan's income trigger |
| declared work area (insured location) | 申報工作地區 | `work_areas`, `match: declared_area`, `areas:` | | Where the declared hours are worked; an event reported per area pays only there (R55) |
| event definition across zones | 跨區事故定義 | `across_zones: true` | | One typhoon is one loss occurrence however many areas announce (R51) |
| accumulation snapshot | 累積額快照 | `AccumulationSnapshot`, `refresh_minutes` | | Benefit in force per zone as last measured; new allocation is checked against it (R51) |
| experience study, actual to expected (A/E) | 經驗研究、實際對預期 | `learning::experience`, `GET /ops/experience` | | Claims incurred against the rating basis's expected cost, by class and period |
| limited-fluctuation credibility | 有限波動可信度 | `credibility_bps`, `full_credibility_claims` | | `Z = min(1, sqrt(n / 1,082))`: how far experience moves the indication |
| rate indication | 費率指示 | `microinsurance_economics.experience` | | A proposed rating basis for the actuary to review and file; never applied by the system (R31) |
| stage gate, scale / hold / stop | 階段關卡 | `config/pilot.yaml`, `pilot_evaluation` | | Gates fixed before the pilot starts (R59) |
| wait-list control group | 候補對照組 | `pilot.control`, `Cohort::WaitList` | | Customers offered prevention tasks later, for measuring claims avoided (R59) |
| renewal notice as issued | 已發出的續保通知 | `RenewalRecord`, `renewals` | | The re-rated default and notice kept as sent; the period opens on it (R35) |
| record of erasure | 刪除紀錄 | `ErasureRecord`, `erasures` | | What the erasure job removed and when (R54) |
| batch job, job queue | 批次作業、作業佇列 | `queue.rs`, `jobs.rs`, `jobs` table | | Claimed with a lease, retried with backoff, failed jobs kept for a person |
