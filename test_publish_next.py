import publish_next


def test_page_offers_both_models():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = []; H.pending = []
    H.initial_max_ts = 0
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
    H.initial_max_ts = 0; H.ai_model = "m"; H.openrouter_model = "o"
    H.config = {"tiers": ["gold"], "galleries": ["Art"]}
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and '>gold<' in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page
