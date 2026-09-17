import re
import sys

KINDS = {
    "Features": "added",
    "Bug Fixes": "fixed",
    "Refactoring": "changed",
    "Breaking Changes": "changed",
    "Performance": "changed",
    "Security": "security",
    "Removed": "removed",
    "Deprecated": "deprecated",
}

PREFIX = re.compile(r"^(?:feat|fix|refactor|docs|chore|style|test|perf|ci|build)(?:\([^)]*\))?!?:\s*")


def entries(changelog, version):
    section = None
    found = []
    for line in changelog.splitlines():
        if line.startswith("## ["):
            if found:
                break
            section = "pending" if line.startswith(f"## [{version}]") else None
            continue
        if section is None:
            continue
        if line.startswith("### "):
            section = line[4:].strip()
            continue
        if line.startswith("- ") and section != "pending":
            found.append((KINDS.get(section, "changed"), PREFIX.sub("", line[2:].strip())))
    return found


def quoted(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    chart_path, changelog_path, version = sys.argv[1], sys.argv[2], sys.argv[3]

    with open(changelog_path, encoding="utf-8") as handle:
        found = entries(handle.read(), version)
    if not found:
        print(f"no changelog entries for {version}, leaving Chart.yaml alone", file=sys.stderr)
        return 0

    with open(chart_path, encoding="utf-8") as handle:
        chart = handle.read()
    if "annotations:" not in chart:
        print("Chart.yaml has no annotations block", file=sys.stderr)
        return 1
    if "artifacthub.io/changes:" in chart:
        print("Chart.yaml already carries artifacthub.io/changes", file=sys.stderr)
        return 1

    block = "\n".join(
        f"    - kind: {kind}\n      description: {quoted(text)}" for kind, text in found
    )
    chart = chart.replace(
        "annotations:\n", f"annotations:\n  artifacthub.io/changes: |\n{block}\n", 1
    )

    with open(chart_path, "w", encoding="utf-8") as handle:
        handle.write(chart)
    print(f"wrote {len(found)} change entries for {version}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
