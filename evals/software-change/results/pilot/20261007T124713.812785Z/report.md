# Campaign pilot

Target: software-change

Dollar figures are estimated from the campaign price table; null means unpriced.

## Classification
Success requires a terminal trial and target success. Timeouts, hangs, limits and model errors are failures. Harness/container/bridge errors and provider-service failures (including exhausted 429/5xx responses) are infrastructure. Unknown exceptions are failures unless the target marks them InfrastructureError. Infrastructure is excluded from success rate and fixed-k pass^k; cases with fewer than k scorable trials are listed as incomplete. No automatic trial retries.

## Settings
```json
{
  "target": "scevals.target:create_target",
  "campaign": "pilot",
  "models": [
    "codex/gpt-6.1-sol",
    "pi/claude-compat/claude-opus-5-5"
  ],
  "combinations": [],
  "preset": "small",
  "case_count": 3,
  "trials_per_case": 1,
  "profile": "standard-14",
  "cases_path": "/Volumes/Workshop/macbook/git/eval-cases",
  "simulator_model": "codex/gpt-6.1-sol",
  "judge_model": "codex/gpt-6.1-sol",
  "idle_limit_seconds": 1800,
  "hard_limit_seconds": 28800,
  "prices": {},
  "model_args": {},
  "model_config": {
    "codex/gpt-6.1-sol": {
      "reasoning_effort": "high"
    },
    "pi/claude-compat/claude-opus-5-5": {
      "reasoning_effort": "medium"
    }
  },
  "provider_config_paths": [
    "/Users/cartwmic/.pi/agent/auth.json"
  ],
  "loop_engine_version": "v0.24.0",
  "provider_version": "v0.24.0",
  "target_options": {},
  "effective_case_count": 3,
  "effective_trials_per_case": 1,
  "resolved_cases_path": "/Volumes/Workshop/macbook/git/eval-cases",
  "resolved_combinations": [
    {
      "id": "codex/gpt-6.1-sol",
      "driver": "codex/gpt-6.1-sol",
      "implementer": "codex/gpt-6.1-sol",
      "reviewer": "codex/gpt-6.1-sol"
    },
    {
      "id": "pi/claude-compat/claude-opus-5-5",
      "driver": "pi/claude-compat/claude-opus-5-5",
      "implementer": "pi/claude-compat/claude-opus-5-5",
      "reviewer": "pi/claude-compat/claude-opus-5-5"
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
    "idle_limit_seconds": "default",
    "hard_limit_seconds": "default",
    "prices": "default",
    "model_args": "default",
    "model_config": "explicit",
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
    "checkout_commit": "3ccb3183bec5e53a6aad577c0bcce8eb16249c4b",
    "checkout_dirty": true
  },
  "target": {
    "loop_engine": "v0.24.0",
    "provider": "v0.24.0",
    "loop_engine_commit": "3e1f9ee924cc1bac3d1924b4868e15697555f3ad",
    "target": "software-change-scorers-v1"
  },
  "eval_cases": {
    "followup-import": "1",
    "recap-preview": "1",
    "review-policy": "1"
  }
}
```

## Matched Assignment
```json
[
  {
    "sample_id": "m1-c2-t1",
    "combination_id": "pi/claude-compat/claude-opus-5-5",
    "case_id": "review-policy",
    "case_version": "1",
    "trial": 1
  }
]
```

## codex/gpt-6.1-sol

### Core metrics
```json
{
  "trial_count": 0,
  "resumed_trials": 0,
  "resumed_success_rate": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "terminated_trials": 0,
  "success_rate": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "pass_k": {
    "k": 1,
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "incomplete_cases": [],
  "infrastructure_failure_rate": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "timeouts": 0,
  "model_errors": 0,
  "failures": 0,
  "workflow_completion": {
    "numerator": 0,
    "denominator": 0,
    "rate": null
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      },
      "role-implementer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      },
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": 0,
    "estimated_usd": null
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
        "estimated_usd": null,
        "price_available": false
      },
      "judge": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": 0,
    "estimated_usd": null
  },
  "wall_seconds": 0
}
```

### Target metrics
```json
{
  "terminal_trials": 0,
  "owner_answers": 0,
  "role_observations": [],
  "software_change_metrics": [],
  "judge_calibration": {
    "status": "calibration pending",
    "judge_model": "codex/gpt-6.1-sol",
    "judge_contract": "intent-judge-v1"
  },
  "metric_basis": "per-trial counts/recall; unavailable external observations are null, not zero"
}
```

## pi/claude-compat/claude-opus-5-5

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
        "model": "pi/claude-compat/claude-opus-5-5",
        "input_tokens": null,
        "output_tokens": null,
        "cache_read_tokens": null,
        "cache_write_tokens": null,
        "total_tokens": null,
        "usage_available": false,
        "estimated_usd": null,
        "price_available": false
      },
      "role-implementer": {
        "model": "pi/claude-compat/claude-opus-5-5",
        "input_tokens": null,
        "output_tokens": null,
        "cache_read_tokens": null,
        "cache_write_tokens": null,
        "total_tokens": null,
        "usage_available": false,
        "estimated_usd": null,
        "price_available": false
      },
      "role-reviewer": {
        "model": "pi/claude-compat/claude-opus-5-5",
        "input_tokens": null,
        "output_tokens": null,
        "cache_read_tokens": null,
        "cache_write_tokens": null,
        "total_tokens": null,
        "usage_available": false,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": null,
    "estimated_usd": null
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": null,
        "output_tokens": null,
        "cache_read_tokens": null,
        "cache_write_tokens": null,
        "total_tokens": null,
        "usage_available": false,
        "estimated_usd": null,
        "price_available": false
      },
      "judge": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 0,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": null,
    "estimated_usd": null
  },
  "wall_seconds": 5.696
}
```

### Target metrics
```json
{
  "terminal_trials": 0,
  "owner_answers": null,
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
        "trial_outcome": "failure"
      },
      "discipline": null,
      "honesty": null,
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
        },
        "role-implementer": {
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
        },
        "role-reviewer": {
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
        }
      },
      "unused_needed_role": null,
      "intent_judge": null
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
    "sample_id": "m1-c2-t1",
    "combination_id": "pi/claude-compat/claude-opus-5-5",
    "case_id": "review-policy",
    "case_version": "1",
    "trial": 1,
    "outcome": "failure",
    "reason": "target_failure",
    "terminal": false,
    "wall_seconds": 5.696,
    "usage_by_role": {},
    "target_metrics": {
      "run_id": "eval-trial",
      "original_usage_available": false,
      "owner_history_available": false,
      "copy_log": [],
      "copy_out_log": [
        {
          "kind": "external_workspace",
          "phase": "after_driver",
          "elapsed_seconds": 0
        }
      ],
      "terminal_state": "design-review",
      "lifecycle": "active",
      "simulated_owner": {
        "model": "codex/gpt-6.1-sol",
        "answer_count": null,
        "answers": null,
        "revealed_requirement_ids": null
      },
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
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
        },
        "role-implementer": {
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
        },
        "role-reviewer": {
          "input_tokens": null,
          "output_tokens": null,
          "total_tokens": null
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
        "trial_outcome": "failure"
      }
    },
    "error_type": null,
    "resumed": false,
    "segments": [],
    "carried_from": null,
    "usage_unknown_roles": [
      "role-driver",
      "role-implementer",
      "role-reviewer",
      "simulator"
    ]
  }
]
```
