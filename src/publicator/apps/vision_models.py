#!/usr/bin/env python3
"""Report which configured local vision models fit this host."""
import argparse
import sys

from publicator.llm_meta import VISION_MODELS, local_model_fit


def main(argv=None) -> int:
    argparse.ArgumentParser(
        description="Report host-fit verdicts for configured local vision models."
    ).parse_args(argv)
    unavailable = False
    for model in VISION_MODELS:
        status = local_model_fit(model)
        level = status["fit_level"]
        if level == "Unavailable":
            label = "unavailable"
            unavailable = True
        else:
            label = "fits" if status["fit"] else "does not fit"
        reason = f" — {status['reason']}" if status["reason"] else ""
        print(f"{model['ollama_name']}: {label} ({level}){reason}")
    return int(unavailable)


if __name__ == "__main__":
    sys.exit(main())
