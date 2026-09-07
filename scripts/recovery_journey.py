"""Focused recovery proof entry points. No unfinished proof is a usable scenario.

Implementers add concrete callables to IMPLEMENTED only once their public path
has terminal assertions. Callables receive the existing Journey instance.
"""

SCENARIOS = (
    "dispositions", "steering", "execution-controls", "cancellation",
    "backtracking", "override", "batched-review", "criteria", "composed-recovery",
)
from recovery_dispositions import prove as prove_dispositions

from recovery_steering import prove as prove_steering

from recovery_execution import prove as prove_execution

from recovery_cancellation import prove as prove_cancellation
from recovery_batch import prove as prove_batch
from recovery_backtracking import prove as prove_backtracking
from recovery_override import prove as prove_override
from recovery_criteria import prove as prove_criteria
from recovery_composed import prove as prove_composed

IMPLEMENTED = {"dispositions": prove_dispositions, "steering": prove_steering, "execution-controls": prove_execution,
               "cancellation": prove_cancellation, "backtracking": prove_backtracking, "batched-review": prove_batch,
               "override": prove_override, "criteria": prove_criteria, "composed-recovery": prove_composed}


def dispatch(name, journey):
    if name not in SCENARIOS:
        raise ValueError(f"unknown recovery scenario: {name}")
    if name not in IMPLEMENTED:
        raise ValueError(f"recovery scenario not implemented: {name}")
    if journey.mode != "source":
        raise ValueError("recovery scenarios require source mode")
    IMPLEMENTED[name](journey)


def self_test():
    # Never substitute a stub success for a planned public proof.
    for name in ("unknown-scenario", *[n for n in SCENARIOS if n not in IMPLEMENTED]):
        try:
            dispatch(name, None)
        except ValueError:
            pass
        else:
            raise AssertionError(f"unfinished/unknown scenario accepted: {name}")
    assert set(IMPLEMENTED) == set(SCENARIOS)
    # Dispatch must propagate a failed public assertion, never print success.
    original = IMPLEMENTED['composed-recovery']
    def failure(_):
        raise AssertionError('deliberate missing terminal observation')
    IMPLEMENTED['composed-recovery'] = failure
    try:
        from types import SimpleNamespace
        try:
            dispatch('composed-recovery', SimpleNamespace(mode='source'))
        except AssertionError as error:
            assert 'missing terminal' in str(error)
        else:
            raise AssertionError('dispatch swallowed public assertion failure')
    finally:
        IMPLEMENTED['composed-recovery'] = original
    assert all(callable(proof) for proof in IMPLEMENTED.values())
