#!/usr/bin/env python3
"""Generate a synthetic tree of a given size, for measuring what a real
repository would only reveal at scale.

Phase A of the roadmap found four scaling walls, and every one of them was
invisible on this repository — 9,700 lines finish in 260 ms, which hides any
asymptotic mistake. Three were outright quadratic (search index build, hub
cohesion, the per-file delta copy) and looked perfectly reasonable in review.

So this exists to be run before claiming a performance property:

    python3 bench/make_tree.py tmp/big 1000000
    glasir serve tmp/big     # cold
    glasir serve tmp/big     # warm

Deterministic: same arguments, same tree, so two measurements are comparable.
The shape is deliberate rather than realistic — many files, shared callees that
give a few nodes very high degree, and a doc comment on every function, since
those are what the walls were made of.
"""

import os
import sys

FUNCS_PER_FILE = 20
FILES_PER_DIR = 50


def main() -> None:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <dir> <lines>", file=sys.stderr)
        raise SystemExit(2)
    root, target = sys.argv[1], int(sys.argv[2])

    # Five lines per function: doc comment, signature, two body lines, close.
    per_file = FUNCS_PER_FILE * 5
    files = max(1, target // per_file)
    dirs = max(1, files // FILES_PER_DIR)

    for d in range(dirs):
        path = os.path.join(root, "src", f"mod{d:03}")
        os.makedirs(path, exist_ok=True)
        for f in range(FILES_PER_DIR):
            body = "\n".join(
                f"/// Handles case {i} of the {d}-{f} path.\n"
                f"fn op_{d}_{f}_{i}(x: u32) -> u32 {{\n"
                # helper_* is shared across the tree: that is what produces the
                # high-degree nodes hub detection has to cope with.
                f"    let y = helper_{(i + 1) % 20}(x);\n"
                f"    other_{d}(y) + {i}\n"
                f"}}"
                for i in range(FUNCS_PER_FILE)
            )
            with open(os.path.join(path, f"f{f:02}.rs"), "w") as out:
                out.write(body + "\n")

    print(f"{dirs * FILES_PER_DIR} files, ~{dirs * FILES_PER_DIR * per_file} lines in {root}")


if __name__ == "__main__":
    main()
