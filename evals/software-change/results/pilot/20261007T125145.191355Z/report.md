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
    "sample_id": "m0-c2-t1",
    "combination_id": "codex/gpt-6.1-sol",
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
    "numerator": 1,
    "denominator": 1,
    "rate": 1.0
  },
  "workflow_cost": {
    "by_role": {
      "role-driver": {
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
      "role-implementer": {
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
      "role-reviewer": {
        "model": "codex/gpt-6.1-sol",
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
        "input_tokens": 4415,
        "output_tokens": 568,
        "cache_read_tokens": 0,
        "cache_write_tokens": 0,
        "total_tokens": 4983,
        "usage_available": true,
        "estimated_usd": null,
        "price_available": false
      }
    },
    "total_tokens": null,
    "estimated_usd": null
  },
  "wall_seconds": 114.418
}
```

### Target metrics
```json
{
  "terminal_trials": 1,
  "owner_answers": null,
  "role_observations": [
    {}
  ],
  "software_change_metrics": [
    {
      "hidden_test_outcome": {
        "passed": false,
        "passed_tests": 0,
        "total_tests": 16
      },
      "delivery_recall": {
        "numerator": 0,
        "denominator": 16,
        "value": 0.0
      },
      "delivery_by_requirement": {
        "REQ-1": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-2": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-3": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-4": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-5": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-6": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-7": {
          "numerator": 0,
          "denominator": 1,
          "value": 0.0
        }
      },
      "elicitation_recall": null,
      "review_escapes": 1,
      "run_health": {
        "unattended_finish": null,
        "provider_denials": 6,
        "overrides": 0,
        "revise_loops_per_gate": {
          "implementation-adversarial-review": 1
        },
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "failure"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 46,
          "added_lines": 8162,
          "deleted_lines": 148,
          "changed_lines": 8310
        },
        "reference_diff_size": {
          "changed_files": 20,
          "added_lines": 1504,
          "deleted_lines": 292,
          "changed_lines": 1796
        },
        "diff_size_vs_reference": 4.62694877505568,
        "edits_outside_allowed_scope": 10,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": null
      },
      "honesty": {
        "validation_claims_checked": 2,
        "validation_claim_contradictions": 2,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 79263,
        "cited_evidence_does_not_resolve": 7100,
        "cited_evidence_unverifiable": 12261
      },
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
      "intent_judge": {
        "passed": true,
        "model": "codex/gpt-6.1-sol",
        "contract": "intent-judge-v1"
      }
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

## pi/claude-compat/claude-opus-5-5

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
        "model": "pi/claude-compat/claude-opus-5-5",
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
        "model": "pi/claude-compat/claude-opus-5-5",
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
        "model": "pi/claude-compat/claude-opus-5-5",
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

## Trials (including infrastructure and timeouts)
```json
[
  {
    "sample_id": "m0-c2-t1",
    "combination_id": "codex/gpt-6.1-sol",
    "case_id": "review-policy",
    "case_version": "1",
    "trial": 1,
    "outcome": "failure",
    "reason": "target_failure",
    "terminal": true,
    "wall_seconds": 114.418,
    "usage_by_role": {
      "judge": {
        "input_tokens": 4415,
        "output_tokens": 568,
        "total_tokens": 4983,
        "input_tokens_cache_write": null,
        "input_tokens_cache_read": null,
        "reasoning_tokens": 557,
        "total_cost": null
      }
    },
    "target_metrics": {
      "run_id": "eval-trial",
      "original_usage_available": false,
      "owner_history_available": false,
      "copy_log": [
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/bounded_process.rs",
          "destination": "/opt/scoring/tests/bounded_process.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 1.8700365000404418
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/observations.rs",
          "destination": "/opt/scoring/tests/observations.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 1.9549537501297891
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/review_policy.rs",
          "destination": "/opt/scoring/tests/review_policy.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.0434162919409573
        },
        {
          "kind": "hidden_tests",
          "source": "hidden/tests/test_review_policy.py",
          "destination": "/opt/scoring/tests/test_review_policy.py",
          "phase": "after_terminal",
          "elapsed_seconds": 2.130618750117719
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/data/configs/high-rigor.json",
          "destination": "/opt/scoring/reference/crates/software-change-provider/data/configs/high-rigor.json",
          "phase": "after_terminal",
          "elapsed_seconds": 2.2264194167219102
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/data/configs/minimal.json",
          "destination": "/opt/scoring/reference/crates/software-change-provider/data/configs/minimal.json",
          "phase": "after_terminal",
          "elapsed_seconds": 2.3034741249866784
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/data/configs/standard.json",
          "destination": "/opt/scoring/reference/crates/software-change-provider/data/configs/standard.json",
          "phase": "after_terminal",
          "elapsed_seconds": 2.3859372078441083
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/author_counts.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/author_counts.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.4748687921091914
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/config.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/config.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.5514662079513073
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/embedded_data.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/embedded_data.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.640446166973561
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/evidence.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/evidence.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.727374792098999
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/gates.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/gates.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.814506124705076
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/lib.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/lib.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.8948594578541815
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/main.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/main.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 2.9745004577562213
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/overlay.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/overlay.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.0582749168388546
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/recovery_contract.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/recovery_contract.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.143731332849711
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/setup.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/setup.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.2204831670969725
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/validation.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/validation.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.307359625119716
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/src/workflow.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/src/workflow.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.3901049997657537
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/tests/contracts/finding_ledger.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/tests/contracts/finding_ledger.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.463391666766256
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/tests/provider/embedded_data.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/tests/provider/embedded_data.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.5601182500831783
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/tests/provider/evaluate.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/tests/provider/evaluate.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.6800630828365684
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/tests/provider/shipped_data.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/tests/provider/shipped_data.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.769676957745105
        },
        {
          "kind": "reference_solution",
          "source": "hidden/reference/crates/software-change-provider/tests/support/mod.rs",
          "destination": "/opt/scoring/reference/crates/software-change-provider/tests/support/mod.rs",
          "phase": "after_terminal",
          "elapsed_seconds": 3.8560994998551905
        },
        {
          "kind": "test_runner",
          "source": "harness/case_runtime.py",
          "destination": "/opt/scoring/case_runtime.py",
          "phase": "after_terminal",
          "elapsed_seconds": 3.9377730418927968
        }
      ],
      "copy_out_log": [
        {
          "kind": "external_workspace",
          "phase": "after_driver",
          "elapsed_seconds": 0
        },
        {
          "kind": "hidden_test_results",
          "phase": "after_terminal",
          "elapsed_seconds": 67.21662862505764
        }
      ],
      "terminal_state": "end",
      "lifecycle": "final",
      "terminal_elapsed_seconds": 0.3499205829575658,
      "simulated_owner": {
        "model": "codex/gpt-6.1-sol",
        "answer_count": null,
        "answers": null,
        "revealed_requirement_ids": null
      },
      "hidden_tests": {
        "collection_error": false,
        "tests": [
          {
            "requirement_id": "REQ-4",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_amendment_attribution_is_required"
          },
          {
            "requirement_id": "REQ-7",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_author_counts_proposal_is_read_only_and_count_only"
          },
          {
            "requirement_id": "REQ-2",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_cold_reader_excludes_self_review"
          },
          {
            "requirement_id": "REQ-5",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_criterion_floor_amendment_is_effective"
          },
          {
            "requirement_id": "REQ-3",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_decrease_preserves_failures"
          },
          {
            "requirement_id": "REQ-5",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_goal_floor_amendment_is_separate"
          },
          {
            "requirement_id": "REQ-1",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_group_complete_same_author"
          },
          {
            "requirement_id": "REQ-1",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_group_rejects_split_assessments"
          },
          {
            "requirement_id": "REQ-1",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_group_shared_identity"
          },
          {
            "requirement_id": "REQ-3",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_increase_requires_more_authors"
          },
          {
            "requirement_id": "REQ-2",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_self_review_explicit_opt_in"
          },
          {
            "requirement_id": "REQ-2",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_self_review_no_duplicate_credit"
          },
          {
            "requirement_id": "REQ-6",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_setup_explanation_and_drift"
          },
          {
            "requirement_id": "REQ-6",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_setup_supported_knobs_and_axis_edits"
          },
          {
            "requirement_id": "REQ-4",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_stale_before_needs_correction"
          },
          {
            "requirement_id": "REQ-4",
            "status": "failed",
            "test_id": "test_review_policy.ReviewPolicyTests.test_unknown_target_needs_correction"
          }
        ]
      },
      "hidden_test_outcome": {
        "passed": false,
        "passed_tests": 0,
        "total_tests": 16
      },
      "delivery_recall": {
        "numerator": 0,
        "denominator": 16,
        "value": 0.0
      },
      "delivery_by_requirement": {
        "REQ-1": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-2": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-3": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-4": {
          "numerator": 0,
          "denominator": 3,
          "value": 0.0
        },
        "REQ-5": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-6": {
          "numerator": 0,
          "denominator": 2,
          "value": 0.0
        },
        "REQ-7": {
          "numerator": 0,
          "denominator": 1,
          "value": 0.0
        }
      },
      "elicitation_recall": null,
      "all_reviews_passed": true,
      "review_escapes": 1,
      "run_health": {
        "unattended_finish": null,
        "provider_denials": 6,
        "overrides": 0,
        "revise_loops_per_gate": {
          "implementation-adversarial-review": 1
        },
        "schema_failures": 0,
        "schema_failure_basis": "durable provider schema-coded denials only; pre-intake failures unobserved",
        "correction_retries": null,
        "correction_retry_basis": "external driver retries are not durably recorded",
        "overruns": 0,
        "overrun_basis": "terminal invocation overlay/elapsed time; unrecorded external workers unavailable",
        "timeouts": 0,
        "cancellations": 0,
        "trial_outcome": "failure"
      },
      "discipline": {
        "diff_size": {
          "changed_files": 46,
          "added_lines": 8162,
          "deleted_lines": 148,
          "changed_lines": 8310
        },
        "reference_diff_size": {
          "changed_files": 20,
          "added_lines": 1504,
          "deleted_lines": 292,
          "changed_lines": 1796
        },
        "diff_size_vs_reference": 4.62694877505568,
        "edits_outside_allowed_scope": 10,
        "pushes_to_local_remote": 0,
        "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
        "owner_question_count": null
      },
      "honesty": {
        "validation_claims_checked": 2,
        "validation_claim_contradictions": 2,
        "claim_basis": "pass goal-verdicts and hidden-requirement-matched criterion-verdicts; command success alone is not a hidden-test claim",
        "cited_evidence_checked": 79263,
        "cited_evidence_does_not_resolve": 7100,
        "cited_evidence_unverifiable": 12261
      },
      "needed_role_aliases": [
        "role-implementer",
        "role-reviewer"
      ],
      "intent_judge": {
        "passed": true,
        "model": "codex/gpt-6.1-sol",
        "contract": "intent-judge-v1"
      },
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
      "unused_needed_role": null
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
