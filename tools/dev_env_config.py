#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
PROFILE_DIR = REPO_ROOT / "dev" / "profiles"
PROFILE_KEY_PATTERN = re.compile(r"^([A-Za-z0-9_]+):(?:[ \t]*(.*))?$")
HOST_PORT_PATTERN = re.compile(r"^(.+):([0-9]+)$")


class ConfigError(Exception):
    pass


def parse_bool(value: str) -> bool:
    normalized = value.strip().lower()
    if normalized in {"1", "true", "yes", "on"}:
        return True
    if normalized in {"0", "false", "no", "off"}:
        return False
    raise ConfigError(f"invalid boolean value: {value!r}")


def parse_int(value: str) -> int:
    try:
        return int(value.strip())
    except ValueError as exc:
        raise ConfigError(f"invalid integer value: {value!r}") from exc


def parse_str(value: str) -> str:
    trimmed = value.strip()
    if trimmed.startswith('"') and trimmed.endswith('"') and len(trimmed) >= 2:
        return trimmed[1:-1]
    if trimmed.startswith("'") and trimmed.endswith("'") and len(trimmed) >= 2:
        return trimmed[1:-1]
    return trimmed


FIELD_PARSERS = {
    "extends": parse_str,
    "cluster_name": parse_str,
    "cluster_context": parse_str,
    "install_namespace": parse_str,
    "registry_name": parse_str,
    "registry_host": parse_str,
    "registry_port": parse_int,
    "image_tag": parse_str,
    "proxy_build_profile": parse_str,
    "test_mode": parse_str,
    "enable_inference_extension": parse_bool,
    "controller_log_level": parse_str,
    "controller_image_repository": parse_str,
    "proxy_image_repository": parse_str,
    "testbox_image_repository": parse_str,
}

ENV_OVERRIDES = {
    "CLUSTER_NAME": ("cluster_name", parse_str),
    "CLUSTER_CONTEXT": ("cluster_context", parse_str),
    "INSTALL_NAMESPACE": ("install_namespace", parse_str),
    "REGISTRY_NAME": ("registry_name", parse_str),
    "REGISTRY_HOST": ("registry_host", parse_str),
    "REGISTRY_PORT": ("registry_port", parse_int),
    "IMAGE_TAG": ("image_tag", parse_str),
    "TAG": ("image_tag", parse_str),
    "PROXY_BUILD_PROFILE": ("proxy_build_profile", parse_str),
    "TEST_MODE": ("test_mode", parse_str),
    "ENABLE_INFERENCE_EXTENSION": ("enable_inference_extension", parse_bool),
    "CONTROLLER_LOG_LEVEL": ("controller_log_level", parse_str),
    "CONTROLLER_IMAGE_REPOSITORY": ("controller_image_repository", parse_str),
    "PROXY_IMAGE_REPOSITORY": ("proxy_image_repository", parse_str),
    "TESTBOX_IMAGE_REPOSITORY": ("testbox_image_repository", parse_str),
}

REQUIRED_FIELDS = {
    "cluster_name",
    "install_namespace",
    "registry_name",
    "registry_host",
    "registry_port",
    "image_tag",
    "proxy_build_profile",
    "test_mode",
    "enable_inference_extension",
    "controller_log_level",
    "controller_image_repository",
    "proxy_image_repository",
    "testbox_image_repository",
}


@dataclass(frozen=True)
class DevEnvConfig:
    profile: str
    cluster_name: str
    cluster_resource_name: str
    cluster_context: str
    install_namespace: str
    registry_name: str
    registry_host: str
    registry_port: int
    registry_host_base: str
    image_tag: str
    proxy_build_profile: str
    test_mode: str
    enable_inference_extension: bool
    controller_log_level: str
    controller_image_repository: str
    proxy_image_repository: str
    testbox_image_repository: str
    registry_name_explicit: bool
    registry_host_explicit: bool
    registry_port_explicit: bool


def profile_path(profile: str) -> Path:
    return PROFILE_DIR / f"{profile}.yaml"


def parse_profile_file(path: Path) -> dict[str, object]:
    if not path.is_file():
        raise ConfigError(f"missing profile file: {path}")

    values: dict[str, object] = {}
    for line_number, raw_line in enumerate(path.read_text().splitlines(), start=1):
        line = raw_line.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        if raw_line[:1].isspace() or line.startswith("-"):
            raise ConfigError(
                f"unsupported profile structure in {path}:{line_number}: {raw_line}"
            )

        match = PROFILE_KEY_PATTERN.match(line)
        if not match:
            raise ConfigError(f"invalid profile line in {path}:{line_number}: {raw_line}")

        key, raw_value = match.groups()
        if key not in FIELD_PARSERS:
            raise ConfigError(f"unknown profile key {key!r} in {path}:{line_number}")
        if key in values:
            raise ConfigError(f"duplicate profile key {key!r} in {path}:{line_number}")

        parser = FIELD_PARSERS[key]
        values[key] = parser(raw_value or "")

    return values


def load_profile(profile: str, seen: set[str] | None = None) -> dict[str, object]:
    if seen is None:
        seen = set()
    if profile in seen:
        raise ConfigError(f"profile inheritance cycle detected at {profile!r}")
    seen.add(profile)

    path = profile_path(profile)
    raw_values = parse_profile_file(path)
    parent_profile = raw_values.pop("extends", None)

    resolved: dict[str, object] = {}
    if parent_profile:
        resolved.update(load_profile(str(parent_profile), seen))
    resolved.update(raw_values)
    return resolved


def resolve_registry(config: dict[str, object], env: dict[str, str]) -> tuple[str, int, str, bool, bool, bool]:
    registry_name_explicit = bool(env.get("REGISTRY_NAME"))
    registry_host_explicit = bool(env.get("REGISTRY_HOST"))
    registry_port_explicit = bool(env.get("REGISTRY_PORT"))

    registry_host = str(config["registry_host"])
    registry_port = int(config["registry_port"])

    match = HOST_PORT_PATTERN.match(registry_host)
    if not match:
        raise ConfigError(
            f"registry_host must be in host:port form, got: {registry_host!r}"
        )
    registry_host_base = match.group(1)
    profile_host_port = int(match.group(2))
    if profile_host_port != registry_port:
        raise ConfigError(
            "registry_host and registry_port disagree in profile: "
            f"{registry_host!r} vs {registry_port}"
        )

    if registry_host_explicit:
        explicit_host = env["REGISTRY_HOST"]
        match = HOST_PORT_PATTERN.match(explicit_host)
        if not match:
            raise ConfigError(
                f"REGISTRY_HOST must be in host:port form, got: {explicit_host!r}"
            )
        registry_host_base = match.group(1)
        explicit_port = int(match.group(2))
        if registry_port_explicit and explicit_port != int(env["REGISTRY_PORT"]):
            raise ConfigError(
                f"REGISTRY_HOST ({explicit_host}) and REGISTRY_PORT ({env['REGISTRY_PORT']}) disagree."
            )
        registry_port = explicit_port
        registry_host = explicit_host
    elif registry_port_explicit:
        registry_port = int(env["REGISTRY_PORT"])
        registry_host = f"{registry_host_base}:{registry_port}"

    return (
        registry_host,
        registry_port,
        registry_host_base,
        registry_name_explicit,
        registry_host_explicit,
        registry_port_explicit,
    )


def resolve_config(profile: str, env: dict[str, str]) -> DevEnvConfig:
    values = load_profile(profile)
    for env_var, (field_name, parser) in ENV_OVERRIDES.items():
        raw_value = env.get(env_var)
        if raw_value:
            values[field_name] = parser(raw_value)

    missing = sorted(REQUIRED_FIELDS - values.keys())
    if missing:
        raise ConfigError(
            f"profile {profile!r} is missing required keys: {', '.join(missing)}"
        )

    cluster_name = str(values["cluster_name"])
    cluster_resource_name = f"kind-{cluster_name}"
    cluster_context = str(values.get("cluster_context") or cluster_resource_name)
    (
        registry_host,
        registry_port,
        registry_host_base,
        registry_name_explicit,
        registry_host_explicit,
        registry_port_explicit,
    ) = resolve_registry(values, env)

    return DevEnvConfig(
        profile=profile,
        cluster_name=cluster_name,
        cluster_resource_name=cluster_resource_name,
        cluster_context=cluster_context,
        install_namespace=str(values["install_namespace"]),
        registry_name=str(values["registry_name"]),
        registry_host=registry_host,
        registry_port=registry_port,
        registry_host_base=registry_host_base,
        image_tag=str(values["image_tag"]),
        proxy_build_profile=str(values["proxy_build_profile"]),
        test_mode=str(values["test_mode"]),
        enable_inference_extension=bool(values["enable_inference_extension"]),
        controller_log_level=str(values["controller_log_level"]),
        controller_image_repository=str(values["controller_image_repository"]),
        proxy_image_repository=str(values["proxy_image_repository"]),
        testbox_image_repository=str(values["testbox_image_repository"]),
        registry_name_explicit=registry_name_explicit,
        registry_host_explicit=registry_host_explicit,
        registry_port_explicit=registry_port_explicit,
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Resolve and validate the shared development environment config."
    )
    parser.add_argument("--profile", default="local", help="profile name under dev/profiles")
    args = parser.parse_args()

    try:
        config = resolve_config(args.profile, dict(os.environ))
    except ConfigError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    json.dump(asdict(config), sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
