"""AC-5 public end-to-end demonstration; real Pi with no-network backends."""
import json

from evalcore.report import utc_timestamp
from evalcore.runner import run_campaign
from evalcore.settings import CampaignSettings
from .target import ROOT, SoftwareChangeTarget


def ac5_demo(args):
    rows = []
    timestamp = utc_timestamp()
    directory = ROOT / "results" / "ac5-demo" / timestamp
    for driver in ("never-asks", "asks-and-builds", "asks-but-does-not-build"):
        settings = CampaignSettings(campaign="ac5-" + timestamp + "-" + driver, models=("sc-scripted/driver",),
                                    cases_path=ROOT / "example-case", case_count=1, trials_per_case=1,
                                    simulator_model="sc-scripted/simulator", judge_model="sc-scripted/judge",
                                    hard_limit_seconds=600, target_options={"scripted_driver": driver})
        destination = run_campaign(SoftwareChangeTarget(), settings, report_directory=directory / driver)
        trial = json.loads((destination / "summary.json").read_text())["trials"][0]
        metrics = trial["target_metrics"]
        if not trial["terminal"] or "delivery_recall" not in metrics:
            raise RuntimeError("scripted trial did not complete scoring")
        rows.append({"driver": driver, "elicitation_recall": metrics["elicitation_recall"],
                     "delivery_recall": metrics["delivery_recall"], "report": str(destination)})
        print(f"{driver}: elicitation recall {metrics['elicitation_recall']['value']:.3f}; "
              f"delivery recall {metrics['delivery_recall']['value']:.3f}")
    never, builds, asks = rows
    passed = (never["elicitation_recall"]["value"] < builds["elicitation_recall"]["value"] == asks["elicitation_recall"]["value"]
              and never["delivery_recall"]["value"] == asks["delivery_recall"]["value"] < builds["delivery_recall"]["value"])
    print("AC5_DEMO " + json.dumps({"passed": passed, "drivers": rows}, sort_keys=True))
    return 0 if passed else 1


def register_subcommands(subparsers):
    parser = subparsers.add_parser("ac5-demo", help="prove separate recall metrics with three real-Pi scripted trials (no paid calls)")
    parser.set_defaults(handler=ac5_demo)
