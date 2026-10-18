# Eval tiers

Each tier is one campaign command. A command runs exactly one campaign and starts no other. Every tier after
Tier 0 needs the owner's explicit go after reading the previous report.

## Tier 0: the cost-and-time floor

- Two models, Sol and Sonnet 5.5, each driving, implementing and reviewing its own trial (the whole workflow).
- The shipped `minimal-16` profile on one trivial case, one trial per model.
- Caps: 30 minutes and 5 dollars for the whole command. The owner can change them in
  `campaigns/tier0.toml` before launch.
- The report states tier, caps, actual spend and time, per-role cost, which stop (if any) fired, and a cost
  verdict. It says Tier 0 is a floor for cost and time. It does not rank the models.

The dollar cap is best effort: spend is polled and a campaign can very rarely overshoot by about one model
call. If it does, the report says so and the cost verdict fails.

## Build-up

Change one thing per tier, in this order, and stop at the first surprise:

1. Complexity: a harder case or a heavier profile (`standard-16`).
2. Cases: more cases.
3. Trials: more trials per case.

Parallel trials, cheaper simulator or judge, caching and early stop are left for later, once a tier shows
where the cost is.
