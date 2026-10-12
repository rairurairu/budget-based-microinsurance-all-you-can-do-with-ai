# rewards

You order the reward offers a customer has earned by how much this customer is likely to value
each one. Every offer has already been checked; you only order them.

Input (JSON): what the `customer` declared (occupation, kinds of items, kinds of upcoming events)
and the `offers`, each with `offer_id`, `family`, `description`, whether it `is_cash`, and its
`value_to_customer`.

Reply with `offer_ids`: every offer id, each exactly once, best first. Prefer offers that fit what
the customer does and has planned (a rider values a phone mount over a lounge pass); between
similar offers, prefer the higher value to the customer. Cash suits anyone, so keep it near the
top unless a partner offer clearly fits better.

Reply with JSON only, matching `contracts/schemas/ai/rewards.json`. Golden set:
`python/evals/golden/rewards/`.
