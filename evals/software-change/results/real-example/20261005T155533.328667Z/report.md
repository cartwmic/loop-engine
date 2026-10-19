# Campaign real-example

Target: software-change

Dollar figures are estimated from the campaign price table; null means unpriced.

## Classification
Success requires a terminal trial and target success. Timeouts, hangs, limits and model errors are failures. Harness/container/bridge errors and provider-service failures (including exhausted 429/5xx responses) are infrastructure. Unknown exceptions are failures unless the target marks them InfrastructureError. Infrastructure is excluded from success rate and fixed-k pass^k; cases with fewer than k scorable trials are listed as incomplete. No automatic trial retries.

## Settings
```json
{
  "target": "scevals.target:create_target",
  "campaign": "real-example",
  "models": [
    "codex/gpt-6.1-sol"
  ],
  "combinations": [],
  "preset": "small",
  "case_count": 1,
  "trials_per_case": 1,
  "profile": "standard-14",
  "cases_path": "evals/software-change/example-case",
  "simulator_model": "openrouter/openai/gpt-6-luna",
  "judge_model": "openrouter/openai/gpt-6-luna",
  "time_limit_seconds": 7200,
  "prices": {
    "openrouter/openai/gpt-6-luna": {
      "input_per_million": 0.1,
      "output_per_million": 0.5,
      "cache_read_per_million": 0.01,
      "cache_write_per_million": 0.125
    }
  },
  "model_args": {
    "openrouter/openai/gpt-6-luna": {
      "strict_tools": false
    }
  },
  "provider_config_paths": [
    "/Users/cartwmic/.pi/agent/auth.json"
  ],
  "loop_engine_version": "v0.24.0",
  "provider_version": "v0.24.0",
  "target_options": {},
  "effective_case_count": 1,
  "effective_trials_per_case": 1,
  "resolved_combinations": [
    {
      "id": "codex/gpt-6.1-sol",
      "driver": "codex/gpt-6.1-sol",
      "implementer": "codex/gpt-6.1-sol",
      "reviewer": "codex/gpt-6.1-sol"
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
    "time_limit_seconds": "explicit",
    "prices": "explicit",
    "model_args": "explicit",
    "provider_config_paths": "default",
    "loop_engine_version": "default",
    "provider_version": "default",
    "target_options": "default"
  }
}
```

## Pinned Models
```json
{
  "simulator": "openrouter/openai/gpt-6-luna",
  "judge": "openrouter/openai/gpt-6-luna"
}
```

## Versions
```json
{
  "harness": {
    "inspect_ai": "0.3.276",
    "evalcore": "0.1.0",
    "python": "3.12.13",
    "checkout_commit": "b68a534756d8b2131a1b13c8b06a713fb934b6ba",
    "checkout_dirty": true
  },
  "target": {
    "loop_engine": "v0.24.0",
    "provider": "v0.24.0",
    "loop_engine_commit": "3e1f9ee924cc1bac3d1924b4868e15697555f3ad",
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
  }
]
```

## codex/gpt-6.1-sol

### Core metrics
```json
{
  "trial_count": 1,
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
  "timeouts": 1,
  "model_errors": 0,
  "failures": 1,
  "workflow_completion": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 5499095,
        "output_tokens": 25036,
        "cache_read_tokens": 2750336,
        "cache_write_tokens": 0,
        "total_tokens": 8274467,
        "estimated_usd": null,
        "price_available": false
      },
      "role-implementer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 684164,
        "output_tokens": 13349,
        "cache_read_tokens": 4224,
        "cache_write_tokens": 0,
        "total_tokens": 701737,
        "estimated_usd": null,
        "price_available": false
      },
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 6763459,
        "output_tokens": 69123,
        "cache_read_tokens": 673920,
        "cache_write_tokens": 0,
        "total_tokens": 7506502,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": 16482706,
    "estimated_usd": null
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "openrouter/openai/gpt-6-luna",
        "input_tokens": 2288,
        "output_tokens": 6105,
        "cache_read_tokens": 0,
        "cache_write_tokens": 10194,
        "total_tokens": 18587,
        "estimated_usd": 0.00455555,
        "price_available": true
      },
      "judge": {
        "model": "openrouter/openai/gpt-6-luna",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "estimated_usd": 0.0,
        "price_available": true
      }
    },
    "total_tokens": 18587,
    "estimated_usd": 0.00455555
  },
  "wall_seconds": 7202.944
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
        "timeouts": 1,
        "cancellations": 0,
        "trial_outcome": "failure"
      },
      "discipline": null,
      "honesty": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 5499095,
          "output_tokens": 25036,
          "total_tokens": 8274467,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2750336,
          "reasoning_tokens": 2888,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 684164,
          "output_tokens": 13349,
          "total_tokens": 701737,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 4224,
          "reasoning_tokens": 314,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 6763459,
          "output_tokens": 69123,
          "total_tokens": 7506502,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 673920,
          "reasoning_tokens": 2043,
          "total_cost": null
        }
      },
      "unused_needed_role": null,
      "intent_judge": null
    }
  ],
  "judge_calibration": {
    "status": "calibration pending",
    "judge_model": "openrouter/openai/gpt-6-luna",
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
    "outcome": "failure",
    "reason": "timeout",
    "terminal": false,
    "wall_seconds": 7202.944,
    "usage_by_role": {
      "role-driver": {
        "input_tokens": 5499095,
        "output_tokens": 25036,
        "total_tokens": 8274467,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 2750336,
        "reasoning_tokens": 2888,
        "total_cost": null
      },
      "simulator": {
        "input_tokens": 2288,
        "output_tokens": 6105,
        "total_tokens": 18587,
        "input_tokens_cache_write": 10194,
        "input_tokens_cache_read": 0,
        "reasoning_tokens": 5332,
        "total_cost": null
      },
      "role-implementer": {
        "input_tokens": 684164,
        "output_tokens": 13349,
        "total_tokens": 701737,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 4224,
        "reasoning_tokens": 314,
        "total_cost": null
      },
      "role-reviewer": {
        "input_tokens": 6763459,
        "output_tokens": 69123,
        "total_tokens": 7506502,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 673920,
        "reasoning_tokens": 2043,
        "total_cost": null
      }
    },
    "target_metrics": {
      "hidden_test_outcome": null,
      "delivery_recall": null,
      "delivery_by_requirement": null,
      "elicitation_recall": null,
      "review_escapes": null,
      "discipline": null,
      "honesty": null,
      "intent_judge": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 5499095,
          "output_tokens": 25036,
          "total_tokens": 8274467,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2750336,
          "reasoning_tokens": 2888,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 684164,
          "output_tokens": 13349,
          "total_tokens": 701737,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 4224,
          "reasoning_tokens": 314,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 6763459,
          "output_tokens": 69123,
          "total_tokens": 7506502,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 673920,
          "reasoning_tokens": 2043,
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
        "timeouts": 1,
        "cancellations": 0,
        "trial_outcome": "failure"
      }
    }
  }
]
```
