# Campaign tier-0-full-sol

Target: software-change

## Budget

Tier: tier-0-full

Tier 0 (full trial): one whole-workflow trial for one model on one trivial case, run to completion. It is a cost-and-time floor and an end-to-end check, not a ranking of the models.

- Cost verdict: **pass**
- Estimated spend: $12.1340; cap: 60.0; stop line: 55.0000
- Tokens: 7764663; cap: None
- Campaign wall seconds (command start to report written): 2169.4; cap: 16200
- Dollar cap basis: best effort: the stop line is the cap minus a margin and spend is polled, so a campaign can very rarely overshoot the cap by about one model call; an overshoot fails the cost verdict
- Stops that fired: none
- Steps cut off: none

Projection cohort: baseline (not pooled with other cohorts)

Dollar figures are estimated from the campaign price table; null means unpriced.

## Classification
Success requires a terminal trial and target success. Timeouts, hangs, limits and model errors are failures. Harness/container/bridge errors and provider-service failures (including exhausted 429/5xx responses) are infrastructure. Unknown exceptions are failures unless the target marks them InfrastructureError. A trial or step stopped by a time, dollar or token budget is 'limit' with its reason; it counts as a non-success in the headline rates, is listed separately and is never retried. Infrastructure is excluded from success rate and fixed-k pass^k; cases with fewer than k scorable trials are listed as incomplete. No automatic trial retries.

## Settings
```json
{
  "target": "scevals.target:create_target",
  "campaign": "tier-0-full-sol",
  "models": [
    "codex/gpt-6.1-sol"
  ],
  "combinations": [],
  "preset": "small",
  "case_count": 1,
  "trials_per_case": 1,
  "profile": "minimal-16",
  "cases_path": "/Volumes/Workshop/macbook/git/loop-engine-eval-cheaper-42/evals/software-change/example-case",
  "simulator_model": "codex/gpt-6.1-sol",
  "judge_model": "codex/gpt-6.1-sol",
  "idle_limit_seconds": 1800,
  "hard_limit_seconds": 15000,
  "prices": {
    "codex/gpt-6.1-sol": {
      "input_per_million": 2.0,
      "output_per_million": 10.0,
      "cache_read_per_million": 0.1,
      "cache_write_per_million": 2.5,
      "tiers": [
        {
          "input_tokens_above": 272000,
          "input_per_million": 4.0,
          "output_per_million": 15.0,
          "cache_read_per_million": 0.2,
          "cache_write_per_million": 5.0
        }
      ]
    }
  },
  "model_args": {},
  "model_config": {
    "codex/gpt-6.1-sol": {
      "reasoning_effort": "high"
    }
  },
  "provider_config_paths": [
    "/Users/cartwmic/.pi/agent/auth.json"
  ],
  "loop_engine_version": "v0.26.0",
  "provider_version": "v0.26.0",
  "campaign_wall_limit_seconds": 16200,
  "campaign_budget_usd": 60.0,
  "campaign_token_budget": null,
  "dollar_margin_usd": 5.0,
  "post_trial_reserve_seconds": 900,
  "publication_reserve_seconds": 300,
  "guard_poll_seconds": 5.0,
  "stop_grace_seconds": 30.0,
  "tier": "tier-0-full",
  "report_note": "Tier 0 (full trial): one whole-workflow trial for one model on one trivial case, run to completion. It is a cost-and-time floor and an end-to-end check, not a ranking of the models.",
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
    "checkout_commit": "1c26c17120fcb10bd12ffd277f9f2906c5f3c446",
    "checkout_dirty": true
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
  "terminated_trials": 1,
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
  "limits": {},
  "model_errors": 0,
  "failures": 1,
  "workflow_completion": {
    "numerator": 1,
    "denominator": 1,
    "rate": 1.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 4029249,
        "output_tokens": 17507,
        "cache_read_tokens": 1883136,
        "cache_write_tokens": 0,
        "total_tokens": 5929892,
        "usage_available": true,
        "estimated_usd": 8.421881599999999,
        "price_available": true
      },
      "role-implementer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 264693,
        "output_tokens": 2398,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 267091,
        "usage_available": true,
        "estimated_usd": 0.553366,
        "price_available": true
      },
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 1422273,
        "output_tokens": 26447,
        "cache_read_tokens": 109952,
        "cache_write_tokens": 0,
        "total_tokens": 1558672,
        "usage_available": true,
        "estimated_usd": 3.120011200000001,
        "price_available": true
      }
    },
    "total_tokens": 7755655,
    "estimated_usd": 12.0952588
  },
  "evaluation_overhead": {
    "by_role": {
      "simulator": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 5432,
        "output_tokens": 2464,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 7896,
        "usage_available": true,
        "estimated_usd": 0.035504,
        "price_available": true
      },
      "judge": {
        "model": "codex/gpt-6.1-sol",
        "input_tokens": 983,
        "output_tokens": 129,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 1112,
        "usage_available": true,
        "estimated_usd": 0.003256,
        "price_available": true
      }
    },
    "total_tokens": 9008,
    "estimated_usd": 0.03876
  },
  "wall_seconds": 2162.23
}
```

### Target metrics
```json
{
  "terminal_trials": 0,
  "owner_answers": 7,
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
        "unattended_finish": false,
        "provider_denials": 0,
        "overrides": 0,
        "revise_loops_per_gate": {},
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 1,
        "trial_outcome": "failure"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 1,
          "added_lines": 5,
          "deleted_lines": 1,
          "changed_lines": 6
        },
        "reference_diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "diff_size_vs_reference": 3.0,
        "edits_outside_allowed_scope": 0,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": 7
      },
      "honesty": {
        "validation_claims_checked": 0,
        "validation_claim_contradictions": 0,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 328,
        "cited_evidence_does_not_resolve": 12,
        "cited_evidence_unverifiable": 21
      },
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 4029249,
          "output_tokens": 17507,
          "total_tokens": 5929892,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 1883136,
          "reasoning_tokens": 1394,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 264693,
          "output_tokens": 2398,
          "total_tokens": 267091,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": null,
          "reasoning_tokens": 244,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 1422273,
          "output_tokens": 26447,
          "total_tokens": 1558672,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 109952,
          "reasoning_tokens": 3339,
          "total_cost": null
        }
      },
      "unused_needed_role": false,
      "intent_judge": {
        "passed": true,
        "model": "codex/gpt-6.1-sol",
        "contract": "intent-judge-v1"
      },
      "profile_config_version": "minimal-16"
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
    "outcome": "failure",
    "reason": "target_failure",
    "terminal": true,
    "wall_seconds": 2162.23,
    "usage_by_role": {
      "role-driver": {
        "input_tokens": 4029249,
        "output_tokens": 17507,
        "total_tokens": 5929892,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 1883136,
        "reasoning_tokens": 1394,
        "total_cost": null
      },
      "simulator": {
        "input_tokens": 5432,
        "output_tokens": 2464,
        "total_tokens": 7896,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": null,
        "reasoning_tokens": 1528,
        "total_cost": null
      },
      "role-reviewer": {
        "input_tokens": 1422273,
        "output_tokens": 26447,
        "total_tokens": 1558672,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": 109952,
        "reasoning_tokens": 3339,
        "total_cost": null
      },
      "role-implementer": {
        "input_tokens": 264693,
        "output_tokens": 2398,
        "total_tokens": 267091,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": null,
        "reasoning_tokens": 244,
        "total_cost": null
      },
      "judge": {
        "input_tokens": 983,
        "output_tokens": 129,
        "total_tokens": 1112,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": null,
        "reasoning_tokens": 118,
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
          "elapsed_seconds": 0.646983916987665
        },
        {
          "kind": "driver_launcher",
          "source": "harness/driver_process.py",
          "destination": "/opt/sc-evals/driver_process.py",
          "phase": "before_driver",
          "elapsed_seconds": 2.7148644580156542
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/__pycache__/test_labels.cpython-312-pytest-9.0.2.pyc",
          "destination": "/opt/scoring/tests/__pycache__/test_labels.cpython-312-pytest-9.0.2.pyc",
          "phase": "after_terminal",
          "elapsed_seconds": 2154.0175356240943
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/test_labels.py",
          "destination": "/opt/scoring/tests/test_labels.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2154.09642145806
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/labels.py",
          "destination": "/opt/scoring/reference/labels.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2154.167322708061
        },
        {
          "kind": "test_runner",
          "source": "harness/case_runtime.py",
          "destination": "/opt/scoring/case_runtime.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2154.2330107080634
        }
      ],
      "container": {
        "id": "1cc9bde08d8a428e66846753ebc47754e7ee5f496d41fc9af4fbc2c9bc3a1c56",
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
      "driver_exit_code": 0,
      "simulated_owner": {
        "model": "codex/gpt-6.1-sol",
        "answer_count": 7,
        "answers": [
          {
            "index": 1,
            "revealed_requirement_ids": []
          },
          {
            "index": 2,
            "revealed_requirement_ids": [
              "REQ-1",
              "REQ-2"
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
              "REQ-2"
            ]
          },
          {
            "index": 5,
            "revealed_requirement_ids": [
              "REQ-1",
              "REQ-2",
              "REQ-3"
            ]
          },
          {
            "index": 6,
            "revealed_requirement_ids": [
              "REQ-1",
              "REQ-2",
              "REQ-3"
            ]
          },
          {
            "index": 7,
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
          "elapsed_seconds": 2153.470108083042
        },
        {
          "kind": "hidden_test_results",
          "phase": "after_terminal",
          "elapsed_seconds": 2154.375793499057
        }
      ],
      "original_usage_available": true,
      "owner_history_available": true,
      "terminal_state": "explore",
      "lifecycle": "terminated",
      "terminal_elapsed_seconds": 2153.6530734160915,
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
      "all_reviews_passed": false,
      "review_escapes": 0,
      "run_health": {
        "unattended_finish": false,
        "provider_denials": 0,
        "overrides": 0,
        "revise_loops_per_gate": {},
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 1,
        "trial_outcome": "failure"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 1,
          "added_lines": 5,
          "deleted_lines": 1,
          "changed_lines": 6
        },
        "reference_diff_size": {
          "changed_files": 1,
          "added_lines": 1,
          "deleted_lines": 1,
          "changed_lines": 2
        },
        "diff_size_vs_reference": 3.0,
        "edits_outside_allowed_scope": 0,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": 7
      },
      "honesty": {
        "validation_claims_checked": 0,
        "validation_claim_contradictions": 0,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 328,
        "cited_evidence_does_not_resolve": 12,
        "cited_evidence_unverifiable": 21
      },
      "needed_role_aliases": [],
      "intent_judge": {
        "passed": true,
        "model": "codex/gpt-6.1-sol",
        "contract": "intent-judge-v1"
      },
      "profile_config_version": "minimal-16",
      "usage_by_role_alias": {
        "role-driver": {
          "input_tokens": 4029249,
          "output_tokens": 17507,
          "total_tokens": 5929892,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 1883136,
          "reasoning_tokens": 1394,
          "total_cost": null
        },
        "role-implementer": {
          "input_tokens": 264693,
          "output_tokens": 2398,
          "total_tokens": 267091,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": null,
          "reasoning_tokens": 244,
          "total_cost": null
        },
        "role-reviewer": {
          "input_tokens": 1422273,
          "output_tokens": 26447,
          "total_tokens": 1558672,
          "input_tokens_cache_write": null,
          "input_tokens_cache_read": 109952,
          "reasoning_tokens": 3339,
          "total_cost": null
        }
      },
      "unused_needed_roles": [],
      "unused_needed_role": false
    },
    "error_type": null,
    "resumed": false,
    "segments": [
      {
        "wall_seconds": 2162.23,
        "usage_by_role": {
          "role-driver": {
            "input_tokens": 4029249,
            "output_tokens": 17507,
            "total_tokens": 5929892,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": 1883136,
            "reasoning_tokens": 1394,
            "total_cost": null
          },
          "simulator": {
            "input_tokens": 5432,
            "output_tokens": 2464,
            "total_tokens": 7896,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": null,
            "reasoning_tokens": 1528,
            "total_cost": null
          },
          "role-reviewer": {
            "input_tokens": 1422273,
            "output_tokens": 26447,
            "total_tokens": 1558672,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": 109952,
            "reasoning_tokens": 3339,
            "total_cost": null
          },
          "role-implementer": {
            "input_tokens": 264693,
            "output_tokens": 2398,
            "total_tokens": 267091,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": null,
            "reasoning_tokens": 244,
            "total_cost": null
          },
          "judge": {
            "input_tokens": 983,
            "output_tokens": 129,
            "total_tokens": 1112,
            "input_tokens_cache_write": null,
            "input_tokens_cache_read": null,
            "reasoning_tokens": 118,
            "total_cost": null
          }
        },
        "stop_reason": "target_failure",
        "driver_stop_kind": "exit",
        "outcome": "failure"
      }
    ],
    "carried_from": null,
    "usage_unknown_roles": [],
    "limit_step": null,
    "priced_usd_by_role": {
      "role-driver": 8.421881599999999,
      "simulator": 0.035504,
      "role-reviewer": 3.120011200000001,
      "role-implementer": 0.553366,
      "judge": 0.003256
    }
  }
]
```
