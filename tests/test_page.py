import urllib.parse

from publicator import publish_next
from publicator.publish_next import GalleryHandler


def _handler(**overrides):
    """Configure the handler's class attributes for a render-only call.
    ponytail: _build_page reads only class attrs, so it is called with the class
    as `self` — constructing a real BaseHTTPRequestHandler would need a socket."""
    GalleryHandler.thumb_dir = "/t"
    GalleryHandler.thumb_map = {}
    GalleryHandler.candidate_paths = []
    GalleryHandler.pending = []
    GalleryHandler.existing_ts = []
    GalleryHandler.ai_model = "m"
    GalleryHandler.openrouter_model = "o"
    GalleryHandler.config = {}
    GalleryHandler.schedules = publish_next.schedule_data({})
    for k, v in overrides.items():
        setattr(GalleryHandler, k, v)
    return GalleryHandler


def test_page_offers_both_models():
    H = _handler(ai_model="claude-cli-opus",
                 openrouter_model="openrouter/google/gemini-2.0-flash-exp:free")
    page = GalleryHandler._build_page(H)
    assert 'id="ai-model"' in page
    assert "claude-cli-opus" in page
    assert "openrouter/google/gemini-2.0-flash-exp:free" in page
    assert "__AI_MODEL__" not in page and "__OPENROUTER_MODEL__" not in page


def test_page_renders_config_tiers_galleries():
    H = _handler(candidate_paths=["/t/a.png"],
                 config={"tiers": ["gold"], "galleries": ["Art"]})
    page = GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and ">gold<" in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    H = _handler(candidate_paths=["/t/a.png"], config=config,
                 schedules=publish_next.schedule_data(config["schedule"]))
    page = GalleryHandler._build_page(H)
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page   # SCHEDULES injected
    assert '"slots"' in page                                       # canonical instants embedded
    assert "__SCHEDULES__" not in page and "__EXISTING_TS__" not in page
    assert "__LABELS__" not in page                                # label map substituted


def test_original_query_decodes_verbatim_for_the_allow_list():
    """/original decodes the path arg verbatim, so the allow-list (path in
    thumb_map) sees the real path — a non-listed path can't sneak through."""
    q = urllib.parse.parse_qs(
        urllib.parse.urlparse("/original?path=%2Fetc%2Fpasswd").query)
    assert q.get("path", [""])[0] == "/etc/passwd"
