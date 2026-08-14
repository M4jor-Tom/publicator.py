"""Publicator: human-review gallery + DeviantArt publishing pipeline."""

import logging


def setup_logging(verbose: bool) -> None:
    """App-wide logging. verbose -> DEBUG (llm calls, HTTP, each publish step),
    else INFO. Shared by every entrypoint; basicConfig is a no-op after the first
    call, so whichever main() runs first wins (they use the same settings)."""
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        datefmt="%H:%M:%S",
    )
