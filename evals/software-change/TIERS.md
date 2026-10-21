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

## Ceilings and caps

Owner rule (Michael, verbatim, 2026-10-21): "don't kill the runs if they go over time. You're just wasting money."

Do not stop a run for going over time. Dollar and time figures in this file (20 dollars API-equivalent and 1 hour per
campaign for the earlier tiers, 60 dollars and 4.5 hours for the two Tier 0 full trials) are reporting targets: the report
states actual spend and time against them. They are not stops. For a run that should not be cut off, leave the per-trial
`hard_limit_seconds` and the campaign wall and dollar caps unset (they are optional; unset means off), and report the
actual spend and time afterwards. A no-progress idle stop (`idle_limit_seconds`) is a stall check, not a time limit, and
stays on. Harness and infrastructure errors still stop the chain.

History. Tier 0 and the first Stage 1 screen did set these caps as stops, and trials that reached them ended as `limit`
(for example the loop-engine arm on `review-policy` at 2400 s of driver time). The Sonnet-only loop-engine rerun sets none.
The Tier 0 full trials were the one exception to the 20 dollar / 1 hour figures; neither cap was reached.

## Build-up

Change one thing per tier, in this order, and stop at the first surprise:

1. Complexity: a harder case or a heavier profile (`standard-16`).
2. Cases: more cases.
3. Trials: more trials per case.

Parallel trials, cheaper simulator or judge, caching and early stop are left for later, once a tier shows
where the cost is.
