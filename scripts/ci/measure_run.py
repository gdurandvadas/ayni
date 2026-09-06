#!/usr/bin/env python3
"""Report comparable GitHub run timings without treating skips as zero-cost success."""
import argparse
from datetime import datetime
import json
import subprocess


def seconds(start: str, end: str) -> float:
    return (datetime.fromisoformat(end.replace("Z", "+00:00")) -
            datetime.fromisoformat(start.replace("Z", "+00:00"))).total_seconds()


def summarize(run: dict, jobs: list[dict], cache_state: str) -> dict:
    if run["status"] != "completed" or any(job["status"] != "completed" for job in jobs):
        raise ValueError("measurement requires a completed run and complete job inventory")
    started = run["run_started_at"]
    finished = max(job["completed_at"] for job in jobs)
    failures = [step["completed_at"] for job in jobs for step in job["steps"]
                if step.get("conclusion") == "failure" and step.get("completed_at")]
    return {
        "run_id": run["id"], "attempt": run["run_attempt"], "source": run["head_sha"],
        "conclusion": run["conclusion"], "cache_state": cache_state,
        "elapsed_seconds": seconds(started, finished),
        "summed_job_seconds": sum(seconds(job["started_at"], job["completed_at"]) for job in jobs),
        "first_failure_seconds": seconds(started, min(failures)) if failures else None,
        "candidate_build_count": sum("build candidate (" in job["name"] for job in jobs),
        "jobs": [{"name": job["name"], "conclusion": job["conclusion"],
                  "seconds": seconds(job["started_at"], job["completed_at"])} for job in jobs],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_id", type=int)
    parser.add_argument("--repository", default="gdurandvadas/ayni")
    parser.add_argument("--cache-state", required=True,
                        help="Observed cache conditions; explicitly identify uncontrolled caches")
    args = parser.parse_args()
    endpoint = f"repos/{args.repository}/actions/runs/{args.run_id}"
    run = json.loads(subprocess.check_output(["gh", "api", endpoint]))
    pages = json.loads(subprocess.check_output([
        "gh", "api", "--paginate", "--slurp", f"{endpoint}/attempts/{run['run_attempt']}/jobs?per_page=100"
    ]))
    jobs = [job for page in pages for job in page["jobs"]]
    print(json.dumps(summarize(run, jobs, args.cache_state), indent=2))


if __name__ == "__main__":
    main()
