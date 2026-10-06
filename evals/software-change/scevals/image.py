"""Pinned source image build and mount-free trial container contract."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
from contextlib import contextmanager

REPOSITORY = Path(__file__).resolve().parents[3]
IMAGE_DIR = Path(__file__).resolve().parents[1] / "image"
DEFAULT_REF = "v0.24.0"
DEFAULT_IMAGE = "loop-engine-sc-evals:v0.24.0"


def run(command, **kwargs):
    return subprocess.run(command, check=True, text=True, **kwargs)


def output(command):
    return run(command, stdout=subprocess.PIPE).stdout.strip()


def build_image(ref=DEFAULT_REF, tag=DEFAULT_IMAGE):
    """Export only the pinned committed tree, never local changes or credentials."""
    arch = output(["docker", "version", "--format", "{{.Server.Arch}}"])
    if arch not in ("arm64", "aarch64"):
        raise RuntimeError("native arm64 Docker is required (no emulation)")
    commit = output(["git", "-C", str(REPOSITORY), "rev-parse", "--verify", f"{ref}^{{commit}}"])
    manifest = output(["git", "-C", str(REPOSITORY), "show", f"{commit}:Cargo.toml"])
    version = tomllib.loads(manifest)["workspace"]["package"]["version"]
    with tempfile.TemporaryDirectory(prefix="sc-image-") as temporary:
        context = Path(temporary)
        for name in ("Dockerfile", "pi-role.py", "ask-owner.py"):
            shutil.copyfile(IMAGE_DIR / name, context / name)
        run(["git", "-C", str(REPOSITORY), "archive", "--format=tar",
             f"--output={context / 'source.tar'}", commit])
        run(["docker", "build", "--platform", "linux/arm64", "--tag", tag,
             "--build-arg", f"LOOP_ENGINE_REF={ref}",
             "--build-arg", f"LOOP_ENGINE_COMMIT={commit}", str(context)])
    return {"ref": ref, "commit": commit, "version": version, "image": tag}


def verify_image(ref=DEFAULT_REF, tag=DEFAULT_IMAGE):
    """Read-only validation; image-check is the only explicit build command."""
    commit = output(["git", "-C", str(REPOSITORY), "rev-parse", "--verify", f"{ref}^{{commit}}"])
    try:
        identity = json.loads(output(["docker", "image", "inspect", tag]))[0]
    except (subprocess.CalledProcessError, ValueError, IndexError):
        raise RuntimeError("trial image missing; run image-check first") from None
    if (identity.get("Architecture") != "arm64" or identity.get("Os") != "linux" or
            (identity["Config"].get("Labels") or {}).get("org.opencontainers.image.revision") != commit or
            identity["Config"].get("Volumes")):
        raise RuntimeError("trial image mismatch; run image-check first")
    return {"ref": ref, "commit": commit, "image": tag}


def trial_container_command(image=DEFAULT_IMAGE):
    """Same creation options used by this check and later direct-Docker trials.

    Inspect adapters must mirror these mount-free options in their compose file.
    Public files enter through docker cp / sandbox.write_file, not host mounts.
    """
    return ["docker", "create", "--platform", "linux/arm64", "--workdir", "/workspace",
            image, "sleep", "infinity"]


@contextmanager
def trial_container(image=DEFAULT_IMAGE):
    container = output(trial_container_command(image))
    try:
        config = json.loads(output(["docker", "inspect", container]))[0]
        if config["Mounts"] or config["HostConfig"].get("Binds") or config["HostConfig"].get("Mounts"):
            raise RuntimeError("trial container must not have mounts")
        run(["docker", "start", container], stdout=subprocess.DEVNULL)
        yield container, config
    finally:
        run(["docker", "rm", "--force", container], stdout=subprocess.DEVNULL)


def check_image(args):
    built = build_image(args.ref, args.tag)
    with trial_container(args.tag) as (container, config):
        versions = {}
        for tool in ("loop-engine", "software-change", "dagu", "pi", "node", "git", "cargo", "rustc"):
            versions[tool] = output(["docker", "exec", container, tool,
                                     "version" if tool == "dagu" else "--version"])
            print(f"{tool}: {versions[tool]}")
        for tool in ("loop-engine", "software-change"):
            match = re.search(r"\b(\d+\.\d+\.\d+(?:[-+][\w.-]+)?)\b", versions[tool])
            if not match or match[1] != built["version"]:
                raise RuntimeError("binary version differs from pinned source version")
        match = re.search(r"\b(\d+)\.(\d+)\.(\d+)\b", versions["dagu"])
        if not match or tuple(map(int, match.groups())) < (2, 14, 0):
            raise RuntimeError("Dagu 2.14.0 or newer is required")
        if not versions["rustc"].startswith("rustc 1.98.0"):
            raise RuntimeError("trial image must carry Rust 1.98.0")
        if versions["pi"] != "1.0.0":
            raise RuntimeError("unexpected pi version")
        image_config = json.loads(output(["docker", "image", "inspect", config["Image"]]))[0]
        if image_config["Architecture"] != "arm64":
            raise RuntimeError("trial image is not native arm64")
        labels = image_config["Config"]["Labels"]
        if labels["org.opencontainers.image.revision"] != built["commit"]:
            raise RuntimeError("image source commit mismatch")
        for role in ("role-driver", "role-implementer", "role-reviewer"):
            models = json.loads(output(["docker", "exec", container, "cat",
                                       f"/opt/sc-evals/pi/{role}/models.json"]))
            if [model["id"] for model in models["providers"]["inspect"]["models"]] != [role]:
                raise RuntimeError("role configuration contains unpinned aliases")
        run(["docker", "exec", container, "sh", "-c",
             "test -s /opt/sc-evals/skills/using-loop-engine/SKILL.md && "
             "test -s /opt/sc-evals/skills/using-software-change-provider/SKILL.md && "
             "for tool in sc-driver sc-implement sc-review ask-owner; do command -v \"$tool\" || exit 1; done"])
        receipt = {**built, "image_id": config["Image"], "architecture": "arm64",
                   "versions": versions, "mounts": config["Mounts"],
                   "binds": config["HostConfig"].get("Binds"),
                   "host_mounts": config["HostConfig"].get("Mounts", [])}
        print("Mounts: " + json.dumps(config["Mounts"]))
        print("IMAGE_CHECK " + json.dumps(receipt, sort_keys=True))
    return 0


def register_subcommands(subparsers: argparse._SubParsersAction):
    parser = subparsers.add_parser("image-check", help="build pinned arm64 trial image and check versions/no mounts")
    parser.add_argument("--ref", default=DEFAULT_REF, help="local Git ref to archive (default v0.24.0)")
    parser.add_argument("--tag", default=DEFAULT_IMAGE, help="local Docker image tag")
    parser.set_defaults(handler=check_image)
