import publish_next


def test_page_offers_both_models():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = []; H.pending = []
    H.existing_ts = []
    H.ai_model = "claude-cli-opus"
    H.openrouter_model = "openrouter/google/gemini-2.0-flash-exp:free"
    # ponytail: _build_page reads only class attrs, so call it with the class as
    # `self` — avoids constructing a real BaseHTTPRequestHandler (needs a socket).
    page = publish_next.GalleryHandler._build_page(H)
    assert 'id="ai-model"' in page
    assert 'claude-cli-opus' in page
    assert 'openrouter/google/gemini-2.0-flash-exp:free' in page
    assert '__AI_MODEL__' not in page and '__OPENROUTER_MODEL__' not in page


def test_page_renders_config_tiers_galleries():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = ["/t/a.png"]; H.pending = []
    H.existing_ts = []; H.ai_model = "m"; H.openrouter_model = "o"
    H.config = {"tiers": ["gold"], "galleries": ["Art"]}
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and '>gold<' in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = ["/t/a.png"]; H.pending = []
    H.existing_ts = []; H.ai_model = "m"; H.openrouter_model = "o"; H.config = {}
    H.schedules = publish_next._schedules_js({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]})
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page   # SCHEDULES injected
    assert '__SCHEDULES__' not in page and '__EXISTING_TS__' not in page
