# Campaign tier-0

Target: software-change

## Budget

Tier: tier-0

Tier 0 is a cost-and-time floor for two whole-workflow models on one trivial case, one trial each. It is not a ranking of the models.

- Cost verdict: **pass**
- Estimated spend: $4.1299; cap: 5.0; stop line: 4.0000
- Tokens: 1255012; cap: None
- Campaign wall seconds (command start to report written): 405.4; cap: 1800
- Dollar cap basis: best effort: the stop line is the cap minus a margin and spend is polled, so a campaign can very rarely overshoot the cap by about one model call; an overshoot fails the cost verdict
- Stops that fired: campaign_dollars at 393.865s
- Steps cut off: not_started (campaign_dollars, m1-c0-t1)

Projection cohort: baseline (not pooled with other cohorts)

Dollar figures are estimated from the campaign price table; null means unpriced.

## Classification
Success requires a terminal trial and target success. Timeouts, hangs, limits and model errors are failures. Harness/container/bridge errors and provider-service failures (including exhausted 429/5xx responses) are infrastructure. Unknown exceptions are failures unless the target marks them InfrastructureError. A trial or step stopped by a time, dollar or token budget is 'limit' with its reason; it counts as a non-success in the headline rates, is listed separately and is never retried. Infrastructure is excluded from success rate and fixed-k pass^k; cases with fewer than k scorable trials are listed as incomplete. No automatic trial retries.

## Settings
```json
{
  "target": "scevals.target:create_target",
  "campaign": "tier-0",
  "models": [
    "codex/gpt-6.1-sol",
    "pi/claude-compat/claude-sonnet-5-5"
  ],
  "combinations": [],
  "preset": "small",
  "case_count": 1,
  "trials_per_case": 1,
  "profile": "minimal-16",
  "cases_path": "evals/software-change/example-case",
  "simulator_model": "codex/gpt-6.1-sol",
  "judge_model": "codex/gpt-6.1-sol",
  "idle_limit_seconds": 300,
  "hard_limit_seconds": 720,
  "prices": {
    "codex/gpt-6.1-sol": {
      "input_per_million": 4.0,
      "output_per_million": 15.0,
      "cache_read_per_million": 0.2,
      "cache_write_per_million": 5.0
    },
    "pi/claude-compat/claude-sonnet-5-5": {
      "input_per_million": 2.0,
      "output_per_million": 10.0,
      "cache_read_per_million": 0.2,
      "cache_write_per_million": 2.5
    }
  },
  "model_args": {},
  "model_config": {
    "codex/gpt-6.1-sol": {
      "reasoning_effort": "high"
    },
    "pi/claude-compat/claude-sonnet-5-5": {
      "reasoning_effort": "high"
    }
  },
  "provider_config_paths": [
    "/Users/cartwmic/.pi/agent/auth.json"
  ],
  "loop_engine_version": "v0.26.0",
  "provider_version": "v0.26.0",
  "campaign_wall_limit_seconds": 1800,
  "campaign_budget_usd": 5.0,
  "campaign_token_budget": null,
  "dollar_margin_usd": 1.0,
  "post_trial_reserve_seconds": 600,
  "publication_reserve_seconds": 120,
  "guard_poll_seconds": 5.0,
  "stop_grace_seconds": 30.0,
  "tier": "tier-0",
  "report_note": "Tier 0 is a cost-and-time floor for two whole-workflow models on one trivial case, one trial each. It is not a ranking of the models.",
  "target_options": {},
  "effective_case_count": 1,
  "effective_trials_per_case": 1,
  "resolved_cases_path": "/Volumes/Workshop/macbook/git/loop-engine-eval-cheaper-42/evals/software-change/example-case",
  "resolved_combinations": [
    {
      "id": "codex/gpt-6.1-sol",
      "driver": "codex/gpt-6.1-sol",
      "implementer": "codex/gpt-6.1-sol",
      "reviewer": "codex/gpt-6.1-sol"
    },
    {
      "id": "pi/claude-compat/claude-sonnet-5-5",
      "driver": "pi/claude-compat/claude-sonnet-5-5",
      "implementer": "pi/claude-compat/claude-sonnet-5-5",
      "reviewer": "pi/claude-compat/claude-sonnet-5-5"
    }
  ],
  "setting_sources": {
    "target": "explicit",
    "campaign": "explicit",
    "models": "explicit",
    "combinations": "default",
    "preset": "default",
    "case_count": "explicit",
    "trials_per_case": "explicit",
    "profile": "explicit",
    "cases_path": "explicit",
    "simulator_model": "explicit",
    "judge_model": "explicit",
    "idle_limit_seconds": "explicit",
    "hard_limit_seconds": "explicit",
    "prices": "explicit",
    "model_args": "default",
    "model_config": "explicit",
    "provider_config_paths": "default",
    "loop_engine_version": "default",
    "provider_version": "default",
    "campaign_wall_limit_seconds": "explicit",
    "campaign_budget_usd": "explicit",
    "campaign_token_budget": "default",
    "dollar_margin_usd": "explicit",
    "post_trial_reserve_seconds": "explicit",
    "publication_reserve_seconds": "explicit",
    "guard_poll_seconds": "default",
    "stop_grace_seconds": "default",
    "tier": "explicit",
    "report_note": "explicit",
    "target_options": "default"
  }
}
```

## Pinned Models
```json
{
  "simulator": "codex/gpt-6.1-sol",
  "judge": "codex/gpt-6.1-sol"
}
```

## Versions
```json
{
  "harness": {
    "inspect_ai": "0.3.276",
    "evalcore": "0.1.0",
    "python": "3.12.13",
    "checkout_commit": "fe116902792621f0759d98b9d68435d922e9a766",
    "checkout_dirty": false
  },
  "target": {
    "loop_engine": "v0.26.0",
    "provider": "v0.26.0",
    "loop_engine_commit": "4699b54d63885a835f27337fc33ad94072e4c939",
    "target": "software-change-scorers-v1"
  },
  "eval_cases": {
    "label-keys": "1"
  }
}
```

## Matched Assignment
```json
[
  {
    "sample_id": "m0-c0-t1",
    "combination_id": "codex/gpt-6.1-sol",
    "case_id": "label-keys",
    "case_version": "1",
    "trial": 1
  },
  {
    "sample_id": "m1-c0-t1",
    "combination_id": "pi/claude-compat/claude-sonnet-5-5",
    "case_id": "label-keys",
    "case_version": "1",
    "trial": 1
  }
]
```

## codex/gpt-6.1-sol

### Core metrics
```json
{
  "trial_count": 1,
  "resumed_trials": 0,
  "resumed_success_rate": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "terminated_trials": 0,
  "success_rate": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "pass_k": {
    "k": 1,
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "incomplete_cases": [],
  "infrastructure_failure_rate": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "timeouts": 0,
  "limits": {
    "campaign_dollars": 1
  },
  "model_errors": 0,
  "failures": 0,
  "workflow_completion": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 810248,
        "output_tokens": 5517,
        "cache_read_tokens": 259456,
        "cache_write_tokens": 0,
        "total_tokens": 1075221,
        "usage_available": true,
        "estimated_usd": 3.3756382,
        "price_available": true
      },
      "role-implementer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      },
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 135950,
        "output_tokens": 5872,
        "cache_read_tokens": 9856,
        "cache_write_tokens": 0,
        "total_tokens": 151678,
        "usage_available": true,
        "estimated_usd": 0.6338512,
        "price_available": true
      }
    },
    "total_tokens": 1226899,
    "estimated_usd": 4.0094894
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 27388,
        "output_tokens": 725,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 28113,
        "usage_available": true,
        "estimated_usd": 0.120427,
        "price_available": true
      },
      "judge": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      }
    },
    "total_tokens": 28113,
    "estimated_usd": 0.120427
  },
  "wall_seconds": 392.716
}
```

### Target metrics
```json
{
  "terminal_trials": 0,
  "owner_answers": 3,
  "role_observations": [
    {}
  ],
  "software_change_metrics": [
    {
      "hidden_test_outcome": null,
      "delivery_recall": null,
      "delivery_by_requirement": null,
      "elicitation_recall": null,
      "review_escapes": null,
      "run_health": {
        "unattended_finish": false,
        "provider_denials": null,
        "overrides": null,
        "revise_loops_per_gate": null,
        "schema_failures": null,
        "correction_retries": null,
        "overruns": null,
        "timeouts": 0,
        "cancellations": 0,
        "timeout_kind": "limit",
        "trial_outcome": "limit"
      },
      "discipline": null,
      "honesty": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 810248,
          "output_tokens": 5517,
          "total_tokens": 1075221,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 259456,
          "reasoning_tokens": 644,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-reviewer": {
          "input_tokens": 135950,
          "output_tokens": 5872,
          "total_tokens": 151678,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 9856,
          "reasoning_tokens": 798,
          "total_cost": null
        }
      },
      "unused_needed_role": null,
      "intent_judge": null,
      "profile_config_version": null
    }
  ],
  "judge_calibration": {
    "status": "calibration pending",
    "judge_model": "codex/gpt-6.1-sol",
    "judge_contract": "intent-judge-v1"
  },
  "metric_basis": "per-trial counts/recall; unavailable external observations are null, not zero"
}
```

## pi/claude-compat/claude-sonnet-5-5

### Core metrics
```json
{
  "trial_count": 1,
  "resumed_trials": 0,
  "resumed_success_rate": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "terminated_trials": 0,
  "success_rate": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "pass_k": {
    "k": 1,
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "incomplete_cases": [],
  "infrastructure_failure_rate": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "timeouts": 0,
  "limits": {
    "campaign_dollars": 1
  },
  "model_errors": 0,
  "failures": 0,
  "workflow_completion": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "pi/claude-compat/claude-sonnet-5-5",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      },
      "role-implementer": {
        "model": "pi/claude-compat/claude-sonnet-5-5",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      },
      "role-reviewer": {
        "model": "pi/claude-compat/claude-sonnet-5-5",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      }
    },
    "total_tokens": 0,
    "estimated_usd": 0.0
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      },
      "judge": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": 0.0,
        "price_available": true
      }
    },
    "total_tokens": 0,
    "estimated_usd": 0.0
  },
  "wall_seconds": 0.686
}
```

### Target metrics
```json
{
  "terminal_trials": 0,
  "owner_answers": 0,
  "role_observations": [
    {}
  ],
  "software_change_metrics": [
    {
      "hidden_test_outcome": null,
      "delivery_recall": null,
      "delivery_by_requirement": null,
      "elicitation_recall": null,
      "review_escapes": null,
      "run_health": {
        "unattended_finish": false,
        "provider_denials": null,
        "overrides": null,
        "revise_loops_per_gate": null,
        "schema_failures": null,
        "correction_retries": null,
        "overruns": null,
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "limit"
      },
      "discipline": null,
      "honesty": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-implementer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-reviewer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        }
      },
      "unused_needed_role": null,
      "intent_judge": null,
      "profile_config_version": null
    }
  ],
  "judge_calibration": {
    "status": "calibration pending",
    "judge_model": "codex/gpt-6.1-sol",
    "judge_contract": "intent-judge-v1"
  },
  "metric_basis": "per-trial counts/recall; unavailable external observations are null, not zero"
}
```

## Trials (including infrastructure and timeouts)
```json
[
  {
    "sample_id": "m0-c0-t1",
    "combination_id": "codex/gpt-6.1-sol",
    "case_id": "label-keys",
    "case_version": "1",
    "trial": 1,
    "outcome": "limit",
    "reason": "campaign_dollars",
    "terminal": false,
    "wall_seconds": 392.716,
    "usage_by_role": {
      "role-driver": {
        "input_tokens": 810248,
        "output_tokens": 5517,
        "total_tokens": 1075221,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 259456,
        "reasoning_tokens": 644,
        "total_cost": null
      },
      "simulator": {
        "input_tokens": 27388,
        "output_tokens": 725,
        "total_tokens": 28113,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": null,
        "reasoning_tokens": 339,
        "total_cost": null
      },
      "role-reviewer": {
        "input_tokens": 135950,
        "output_tokens": 5872,
        "total_tokens": 151678,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 9856,
        "reasoning_tokens": 798,
        "total_cost": null
      }
    },
    "target_metrics": {
      "copy_log": [
        {
          "kind": "public",
          "source": "public/",
          "destination": "/workspace/public",
          "phase": "before_driver",
          "elapsed_seconds": 0.6969558330019936
        },
        {
          "kind": "driver_launcher",
          "source": "harness/driver_process.py",
          "destination": "/opt/sc-evals/driver_process.py",
          "phase": "before_driver",
          "elapsed_seconds": 2.902932957978919
        }
      ],
      "container": {
        "id": "c4537a7d18e975ebeebe5a3e7e9efa9dd7a77d83197491d43082344ba29c9eea",
        "image_id": "sha256:1fa3e110e421150bce25517a4e69c63fac9ab5778437949ed71468d9c36f7bfe",
        "mounts": [],
        "binds": [],
        "platform": "linux",
        "working_dir": "/workspace"
      },
      "role_models": {
        "role-driver": "codex/gpt-6.1-sol",
        "role-implementer": "codex/gpt-6.1-sol",
        "role-reviewer": "codex/gpt-6.1-sol"
      },
      "run_id": "eval-trial",
      "driver_timed_out": true,
      "driver_timeout_kind": "limit",
      "driver_exit_code": 247,
      "simulated_owner": {
        "model": "codex/gpt-6.1-sol",
        "answer_count": 3,
        "answers": [
          {
            "index": 1,
            "revealed_requirement_ids": []
          },
          {
            "index": 2,
            "revealed_requirement_ids": [
              "REQ-1",
              "REQ-2",
              "REQ-3"
            ]
          },
          {
            "index": 3,
            "revealed_requirement_ids": []
          }
        ],
        "revealed_requirement_ids": [
          "REQ-1",
          "REQ-2",
          "REQ-3"
        ]
      },
      "scripted_driver": null,
      "copy_out_log": [
        {
          "kind": "workspace",
          "phase": "after_driver",
          "elapsed_seconds": 388.6288789159735
        }
      ],
      "original_usage_available": true,
      "owner_history_available": true,
      "terminal_state": "intent-review",
      "lifecycle": "active",
      "hidden_test_outcome": null,
      "delivery_recall": null,
      "delivery_by_requirement": null,
      "elicitation_recall": null,
      "review_escapes": null,
      "discipline": null,
      "honesty": null,
      "intent_judge": null,
      "profile_config_version": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 810248,
          "output_tokens": 5517,
          "total_tokens": 1075221,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 259456,
          "reasoning_tokens": 644,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-reviewer": {
          "input_tokens": 135950,
          "output_tokens": 5872,
          "total_tokens": 151678,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 9856,
          "reasoning_tokens": 798,
          "total_cost": null
        }
      },
      "unused_needed_roles": null,
      "unused_needed_role": null,
      "run_health": {
        "unattended_finish": false,
        "provider_denials": null,
        "overrides": null,
        "revise_loops_per_gate": null,
        "schema_failures": null,
        "correction_retries": null,
        "overruns": null,
        "timeouts": 0,
        "cancellations": 0,
        "timeout_kind": "limit",
        "trial_outcome": "limit"
      }
    },
    "error_type": null,
    "resumed": false,
    "segments": [
      {
        "wall_seconds": 392.716,
        "usage_by_role": {
          "role-driver": {
            "input_tokens": 810248,
            "output_tokens": 5517,
            "total_tokens": 1075221,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": 259456,
            "reasoning_tokens": 644,
            "total_cost": null
          },
          "simulator": {
            "input_tokens": 27388,
            "output_tokens": 725,
            "total_tokens": 28113,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": null,
            "reasoning_tokens": 339,
            "total_cost": null
          },
          "role-reviewer": {
            "input_tokens": 135950,
            "output_tokens": 5872,
            "total_tokens": 151678,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": 9856,
            "reasoning_tokens": 798,
            "total_cost": null
          }
        },
        "stop_reason": "campaign_dollars",
        "driver_stop_kind": "limit",
        "outcome": "limit"
      }
    ],
    "carried_from": null,
    "usage_unknown_roles": [],
    "limit_step": null
  },
  {
    "sample_id": "m1-c0-t1",
    "combination_id": "pi/claude-compat/claude-sonnet-5-5",
    "case_id": "label-keys",
    "case_version": "1",
    "trial": 1,
    "outcome": "limit",
    "reason": "campaign_dollars",
    "terminal": false,
    "wall_seconds": 0.686,
    "usage_by_role": {},
    "target_metrics": {
      "hidden_test_outcome": null,
      "delivery_recall": null,
      "delivery_by_requirement": null,
      "elicitation_recall": null,
      "review_escapes": null,
      "discipline": null,
      "honesty": null,
      "intent_judge": null,
      "profile_config_version": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-implementer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        },
        "role-reviewer": {
          "input_tokens": 0,
          "output_tokens": 0,
          "total_tokens": 0
        }
      },
      "unused_needed_roles": null,
      "unused_needed_role": null,
      "run_health": {
        "unattended_finish": false,
        "provider_denials": null,
        "overrides": null,
        "revise_loops_per_gate": null,
        "schema_failures": null,
        "correction_retries": null,
        "overruns": null,
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "limit"
      }
    },
    "error_type": "LimitReached",
    "resumed": false,
    "segments": [],
    "carried_from": null,
    "usage_unknown_roles": [],
    "limit_step": "not_started"
  }
]
```
