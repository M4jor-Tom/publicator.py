"""Live OpenRouter check: metadata really comes back over the schema fallback.

Gated on connectivity, not on the key — reachable network with no $OPENROUTER_KEY
is a *failure* (the key is missing, fix it); no network skips with a warning.
The model is deliberately one whose OpenRouter `supported_parameters` lack
`structured_outputs`, so this exercises llm_meta's fallback rather than --schema.
"""
import base64
import os
import socket
import warnings

import pytest

from publicator import llm_meta

# A model that does NOT advertise structured_outputs, which is the point. Free ids
# churn: if this one 404s the fallback isn't broken, the id is — pick another from
#   curl -s https://openrouter.ai/api/v1/models | jq -r '.data[]
#     | select(.id|endswith(":free")) | select(.architecture.input_modalities|index("image"))
#     | select(.supported_parameters|index("structured_outputs")|not) | .id'
SCHEMALESS_VISION_MODEL = "openrouter/nvidia/nemotron-nano-12b-v2-vl:free"

# 48x48 navy->orange gradient, so the model has something real to describe.
# Regenerate with: magick -size 48x48 gradient:navy-orange -strip x.png | base64
GRADIENT_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAADAAAAAwEAIAAACI8LKTAAABTElEQVRo3u3aoUqEQRQF4H/8zx0X0eKywe"
    "IKYjQIPoL6FHarzERBNq1Fm80mLBjEbLBpF6NFDG42GRR0DZv/e+O94XxPcMLMYe7MpKZpmtGooQ6Q0o5/"
    "97xjxJWWpovLJz3vGHEhl/aMK6gbpIBbTIFc2UGatHE32Dpe844RF3JlB2nYQYa0I8P7o6l3jLggFeM/rq"
    "BOEJ6DVMjsIFU6eN5+OfzyjhEXcmEHadhBBuTKDtJASsstppivoH3vGHHxRtEAKWBJK5A5aqi4xQwcNQwQ"
    "XpipOGoY2EEGjhoGDqsGCDtINX845CzWic8+BuTK6w4Nh1UDMs9BKgjPQar0NDh9X+97x4iLHWSA8AOViv"
    "dBhvT6dv7d3/SOERdyZQdp0sfjZV659Y4RF3+5GtLnzdVqb9c7Rlz8aW9IPxfXw4UH7xhxpdlsMvEOEdk/"
    "pqFvwV/P10QAAAAASUVORK5CYII="
)


def _online(host="openrouter.ai", port=443, timeout=3):
    try:
        socket.create_connection((host, port), timeout).close()
        return True
    except OSError:
        return False


def test_openrouter_generates_title_and_description(tmp_path):
    if not _online():
        # warn, not just skip: a silent `s` is how a live check rots unnoticed.
        warnings.warn("no internet: skipped the live OpenRouter generation check")
        pytest.skip("no internet")
    assert os.environ.get("OPENROUTER_KEY"), (
        "OPENROUTER_KEY is unset but the network is up — source your key script "
        "(this test only skips when offline)"
    )
    art = tmp_path / "art.png"
    art.write_bytes(GRADIENT_PNG)

    # The free pool hiccups (429s, upstream idle timeouts) often enough that one
    # shot is a coin flip. Retry so a red run means OUR bug, not OpenRouter's weather.
    for _ in range(3):
        try:
            title, description = llm_meta.generate_metadata(
                str(art), SCHEMALESS_VISION_MODEL, timeout=90)
            break
        except RuntimeError as e:
            last = e
    else:
        pytest.fail(f"3 attempts against {SCHEMALESS_VISION_MODEL} failed; last: {last}")

    assert 0 < len(title) <= 120
    assert len(description.split()) >= 5
    # JSON was parsed, not echoed through as prose
    assert "{" not in description and "```" not in description
