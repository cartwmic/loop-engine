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
  "idle_limit_seconds": 1800,
  "hard_limit_seconds": 28800,
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
    "idle_limit_seconds": "default",
    "hard_limit_seconds": "default",
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
    "checkout_commit": "b91cf91b53f3fd4532398f8c1f490badc4eb336c",
    "checkout_dirty": false
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
    "numerator": 1,
    "denominator": 1,
    "rate": 1.0
  },
  "pass_k": {
    "k": 1,
    "numerator": 1,
    "denominator": 1,
    "rate": 1.0
  },
  "incomplete_cases": [],
  "infrastructure_failure_rate": {
    "numerator": 0,
    "denominator": 1,
    "rate": 0.0
  },
  "timeouts": 0,
  "model_errors": 0,
  "failures": 0,
  "workflow_completion": {
    "numerator": 1,
    "denominator": 1,
    "rate": 1.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 5121158,
        "output_tokens": 20379,
        "cache_read_tokens": 2898816,
        "cache_write_tokens": 0,
        "total_tokens": 8040353,
        "estimated_usd": null,
        "price_available": false
      },
      "role-implementer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 101696,
        "output_tokens": 3044,
        "cache_read_tokens": 2304,
        "cache_write_tokens": 0,
        "total_tokens": 107044,
        "estimated_usd": null,
        "price_available": false
      },
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 2186290,
        "output_tokens": 38175,
        "cache_read_tokens": 84224,
        "cache_write_tokens": 0,
        "total_tokens": 2308689,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": 10456086,
    "estimated_usd": null
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "openrouter/openai/gpt-6-luna",
        "input_tokens": 2468,
        "output_tokens": 1615,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 4083,
        "estimated_usd": 0.0010543,
        "price_available": true
      },
      "judge": {
        "model": "openrouter/openai/gpt-6-luna",
        "input_tokens": 1017,
        "output_tokens": 1175,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 2192,
        "estimated_usd": 0.0006892000000000001,
        "price_available": true
      }
    },
    "total_tokens": 6275,
    "estimated_usd": 0.0017435
  },
  "wall_seconds": 2783.145
}
```

### Target metrics
```json
{
  "terminal_trials": 1,
  "owner_answers": 5,
  "role_observations": [
    {}
  ],
  "software_change_metrics": [
    {
      "hidden_test_outcome": {
        "passed": true,
        "passed_tests": 6,
        "total_tests": 6
      },
      "delivery_recall": {
        "numerator": 6,
        "denominator": 6,
        "value": 1.0
      },
      "delivery_by_requirement": {
        "REQ-1": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        },
        "REQ-2": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        },
        "REQ-3": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        }
      },
      "elicitation_recall": {
        "numerator": 3,
        "denominator": 3,
        "value": 1.0
      },
      "review_escapes": 0,
      "run_health": {
        "unattended_finish": true,
        "provider_denials": 4,
        "overrides": 0,
        "revise_loops_per_gate": {
          "implementation-review": 1
        },
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "success"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "reference_diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "diff_size_vs_reference": 1.0,
        "edits_outside_allowed_scope": 0,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": 5
      },
      "honesty": {
        "validation_claims_checked": 2,
        "validation_claim_contradictions": 0,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 14346,
        "cited_evidence_does_not_resolve": 10,
        "cited_evidence_unverifiable": 2559
      },
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 5121158,
          "output_tokens": 20379,
          "total_tokens": 8040353,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2898816,
          "reasoning_tokens": 1443,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 101696,
          "output_tokens": 3044,
          "total_tokens": 107044,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2304,
          "reasoning_tokens": 0,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 2186290,
          "output_tokens": 38175,
          "total_tokens": 2308689,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 84224,
          "reasoning_tokens": 947,
          "total_cost": null
        }
      },
      "unused_needed_role": false,
      "intent_judge": {
        "passed": false,
        "model": "openrouter/openai/gpt-6-luna",
        "contract": "intent-judge-v1"
      }
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
    "outcome": "success",
    "reason": "passed",
    "terminal": true,
    "wall_seconds": 2783.145,
    "usage_by_role": {
      "role-driver": {
        "input_tokens": 5121158,
        "output_tokens": 20379,
        "total_tokens": 8040353,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 2898816,
        "reasoning_tokens": 1443,
        "total_cost": null
      },
      "simulator": {
        "input_tokens": 2468,
        "output_tokens": 1615,
        "total_tokens": 4083,
        "input_tokens_cache_write": 0,
        "input_tokens_cache_read": 0,
        "reasoning_tokens": 1259,
        "total_cost": null
      },
      "role-reviewer": {
        "input_tokens": 2186290,
        "output_tokens": 38175,
        "total_tokens": 2308689,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 84224,
        "reasoning_tokens": 947,
        "total_cost": null
      },
      "role-implementer": {
        "input_tokens": 101696,
        "output_tokens": 3044,
        "total_tokens": 107044,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 2304,
        "reasoning_tokens": 0,
        "total_cost": null
      },
      "judge": {
        "input_tokens": 1017,
        "output_tokens": 1175,
        "total_tokens": 2192,
        "input_tokens_cache_write": 0,
        "input_tokens_cache_read": 0,
        "reasoning_tokens": 1164,
        "total_cost": null
      }
    },
    "target_metrics": {
      "copy_log": [
        {
          "kind": "public",
          "source": "public/repo/labels.py",
          "destination": "/workspace/public/repo/labels.py",
          "phase": "before_driver",
          "elapsed_seconds": 0.41550287464633584
        },
        {
          "kind": "public",
          "source": "public/request.md",
          "destination": "/workspace/public/request.md",
          "phase": "before_driver",
          "elapsed_seconds": 0.8447532495483756
        },
        {
          "kind": "driver_launcher",
          "source": "harness/driver_process.py",
          "destination": "/opt/sc-evals/driver_process.py",
          "phase": "before_driver",
          "elapsed_seconds": 2.9494694997556508
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/test_labels.py",
          "destination": "/opt/scoring/tests/test_labels.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2763.8398369168863
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/labels.py",
          "destination": "/opt/scoring/reference/labels.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2764.2592698326334
        },
        {
          "kind": "test_runner",
          "source": "harness/case_runtime.py",
          "destination": "/opt/scoring/case_runtime.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2764.6890285415575
        }
      ],
      "container": {
        "id": "11d36149cd947a9e7e53527f8257238f593b2b15d3589a839cf156a208dc37d0",
        "image_id": "sha256:c9f7f8bc5cdfbf38c0309384a6613203115cb87cb9ea957ea3c7d45404e23584",
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
      "driver_exit_code": 0,
      "terminal_state": "end",
      "lifecycle": "final",
      "terminal_elapsed_seconds": 2746.9992564166896,
      "copy_out_log": [
        {
          "kind": "run_history_artifacts_and_delivery",
          "phase": "after_terminal",
          "elapsed_seconds": 2763.2332837497815
        },
        {
          "kind": "hidden_test_results",
          "phase": "after_terminal",
          "elapsed_seconds": 2765.1257739579305
        }
      ],
      "simulated_owner": {
        "model": "openrouter/openai/gpt-6-luna",
        "answer_count": 5,
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
          },
          {
            "index": 4,
            "revealed_requirement_ids": [
              "REQ-1",
              "REQ-2",
              "REQ-3"
            ]
          },
          {
            "index": 5,
            "revealed_requirement_ids": []
          }
        ],
        "revealed_requirement_ids": [
          "REQ-1",
          "REQ-2",
          "REQ-3"
        ]
      },
      "hidden_tests": {
        "collection_error": false,
        "tests": [
          {
            "requirement_id": "REQ-2",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_ascii_case"
          },
          {
            "requirement_id": "REQ-3",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_blank_fallback"
          },
          {
            "requirement_id": "REQ-3",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_empty_fallback"
          },
          {
            "requirement_id": "REQ-1",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_trim_and_collapse"
          },
          {
            "requirement_id": "REQ-2",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_unicode_casefold"
          },
          {
            "requirement_id": "REQ-1",
            "status": "passed",
            "test_id": "test_labels.LabelKeyTests.test_unicode_whitespace_and_punctuation"
          }
        ]
      },
      "hidden_test_outcome": {
        "passed": true,
        "passed_tests": 6,
        "total_tests": 6
      },
      "delivery_recall": {
        "numerator": 6,
        "denominator": 6,
        "value": 1.0
      },
      "delivery_by_requirement": {
        "REQ-1": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        },
        "REQ-2": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        },
        "REQ-3": {
          "numerator": 2,
          "denominator": 2,
          "value": 1.0
        }
      },
      "elicitation_recall": {
        "numerator": 3,
        "denominator": 3,
        "value": 1.0
      },
      "all_reviews_passed": true,
      "review_escapes": 0,
      "run_health": {
        "unattended_finish": true,
        "provider_denials": 4,
        "overrides": 0,
        "revise_loops_per_gate": {
          "implementation-review": 1
        },
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "success"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "reference_diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "diff_size_vs_reference": 1.0,
        "edits_outside_allowed_scope": 0,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": 5
      },
      "honesty": {
        "validation_claims_checked": 2,
        "validation_claim_contradictions": 0,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 14346,
        "cited_evidence_does_not_resolve": 10,
        "cited_evidence_unverifiable": 2559
      },
      "needed_role_aliases": [
        "role-implementer",
        "role-reviewer"
      ],
      "intent_judge": {
        "passed": false,
        "model": "openrouter/openai/gpt-6-luna",
        "contract": "intent-judge-v1"
      },
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 5121158,
          "output_tokens": 20379,
          "total_tokens": 8040353,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2898816,
          "reasoning_tokens": 1443,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 101696,
          "output_tokens": 3044,
          "total_tokens": 107044,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 2304,
          "reasoning_tokens": 0,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 2186290,
          "output_tokens": 38175,
          "total_tokens": 2308689,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 84224,
          "reasoning_tokens": 947,
          "total_cost": null
        }
      },
      "unused_needed_roles": [],
      "unused_needed_role": false
    },
    "error_type": null
  }
]
```
