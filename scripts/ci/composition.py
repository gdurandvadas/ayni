#!/usr/bin/env python3
"""Materialize real polyglot repositories from the locked canonical examples."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import shutil
import subprocess

from validate_example_artifact import EXPECTED_OUTCOMES, validate_artifact

CASES = {
    "all-five": ["rust", "node", "go", "python", "kotlin"],
    "rust-node": ["rust", "node"],
    "kotlin-go": ["kotlin", "go"],
}


def expected_targets(case: str) -> dict[str, str]:
    targets = {language: language for language in CASES[case]}
    if case == "rust-node":
        targets["node-other"] = "node"
    return targets


def materialize(source: Path, destination: Path, case: str) -> dict[str, str]:
    languages = CASES[case]
    destination.mkdir(parents=True, exist_ok=False)
    targets = expected_targets(case)
    # A second independent native workspace exercises multiple roots, including
    # preparation reuse when only one workspace's dependency inputs change.
    sections = []
    for root, language in targets.items():
        example = Path("examples") / language / "mono"
        files = subprocess.check_output(
            ["git", "-C", str(source), "ls-files", "-z", str(example)]
        ).decode().split("\0")
        for name in filter(None, files):
            relative = Path(name).relative_to(example)
            if relative.name in (".ayni.toml", ".ayni.lock"):
                continue
            output = destination / root / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / name, output)
        if root == language:
            config = (source / example / ".ayni.toml").read_text()
            config = f"[{language}]" + config.split(f"[{language}]", 1)[1]
            roots = [name for name, owner in targets.items() if owner == language]
            sections.append(config.replace('roots = ["."]', f"roots = {json.dumps(roots)}", 1))
    config = """[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false
[concurrency]
amount = 1
[report]
offenders_limit = 100
[environment.tools]
jq = "1.7.1"
"""
    config += f"[languages]\nenabled = {json.dumps(languages)}\n"
    if "rust" in languages:
        config += '[environment.debian]\npackages = ["libssl-dev"]\n'
    (destination / ".ayni.toml").write_text(config + "\n".join(sections))
    interactions = destination / "interactions"
    interactions.mkdir()
    if "rust" in languages:
        (interactions / "rust-node.rs").write_text('''fn main() {
    let result = std::process::Command::new("node")
        .args(["-e", "process.stdout.write('node-from-rust')"]).output().unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"node-from-rust");
}
''')
    if "kotlin" in languages:
        (interactions / "main.go").write_text('''package main
import ("fmt"; "os/exec")
func main() {
    if err := exec.Command("java", "-version").Run(); err != nil { panic(err) }
    fmt.Print("go-from-kotlin")
}
''')
        test = destination / "kotlin/libs/greeting/src/test/kotlin/ayni/greeting/CompositionTest.kt"
        test.write_text('''package ayni.greeting
import kotlin.test.Test
import kotlin.test.assertEquals
class CompositionTest {
    @Test fun invokesGoAndJava() {
        val process = ProcessBuilder("go", "run", "/workspace/interactions/main.go").start()
        val output = process.inputStream.bufferedReader().readText()
        assertEquals(0, process.waitFor())
        assertEquals("go-from-kotlin", output)
    }
}
''')
    subprocess.run(["git", "init", "--quiet", str(destination)], check=True)
    subprocess.run(["git", "-C", str(destination), "add", "."], check=True)
    return targets


def validate(document: dict, targets: dict[str, str], exit_code: int) -> None:
    """Check global accounting and each target against its real mono contract."""
    expected = {(language, root) for root, language in targets.items()}
    rows = document["rows"]
    actual = {(row["language"], row["scope"].get("path", ".")) for row in rows}
    if actual != expected or set(document["invocation"]["languages"]) != set(targets.values()):
        raise ValueError("composition omitted or introduced targets/languages")
    completion = document["completion"]
    if any(completion.get(key) != len(targets) for key in
           ("expected_targets", "detected_targets", "completed_targets")):
        raise ValueError("composition target accounting is incomplete")
    expected_exit = int(any(EXPECTED_OUTCOMES[language].check_exit_code for language in targets.values()))
    if exit_code != expected_exit:
        raise ValueError("composition exit code lost aggregate failures")
    counts = dict(total_rows=0, passing_rows=0, failing_rows=0, warning_offenders=0, failing_offenders=0)
    for root, language in targets.items():
        projected = copy.deepcopy(document)
        projected["invocation"]["languages"] = [language]
        for key in ("expected_targets", "detected_targets", "completed_targets"):
            projected["completion"][key] = 1
        for key in ("rows", "applied_thresholds", "offender_summaries"):
            projected[key] = [row for row in projected[key]
                              if row["language"] == language and row["scope"].get("path", ".") == root]
        selected = projected["rows"]
        passing = sum(row["pass"] for row in selected)
        offenders = [item for row in selected for item in row["offenders"]["items"]]
        projected["aggregate"] = {
            "status": "pass" if passing == len(selected) else "fail",
            "total_rows": len(selected), "passing_rows": passing,
            "failing_rows": len(selected) - passing,
            "warning_offenders": sum(item.get("level") == "warn" for item in offenders),
            "failing_offenders": sum(item.get("level") == "fail" for item in offenders),
        }
        validate_artifact(projected, language=language, expected_roots=[root],
                          repository_root="/workspace", check_exit_code=EXPECTED_OUTCOMES[language].check_exit_code)
        for key in counts:
            counts[key] += projected["aggregate"][key]
    counts["status"] = "fail" if expected_exit else "pass"
    if any(document["aggregate"].get(key) != value for key, value in counts.items()):
        raise ValueError("composition aggregate differs from its target evidence")
    for key in ("applied_thresholds", "offender_summaries"):
        if any((row["language"], row["scope"].get("path", ".")) not in expected for row in document[key]):
            raise ValueError("composition summary contains an unknown target")
