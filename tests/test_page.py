from publicator.scheduling import schedule_data
from publicator.webui.page import render_page


def _render(**overrides):
    kwargs = dict(thumb_dir="/t", thumb_map={}, candidates=[], pending=[],
                  existing_ts=[], schedules=schedule_data({}), config={},
                  ai_model="m", openrouter_model="o")
    kwargs.update(overrides)
    return render_page(**kwargs)


def test_page_offers_both_models():
    page = _render(ai_model="claude-cli-opus",
                   openrouter_model="openrouter/google/gemini-2.0-flash-exp:free")
    assert 'id="ai-model"' in page
    assert "claude-cli-opus" in page
    assert "openrouter/google/gemini-2.0-flash-exp:free" in page
    assert "__AI_MODEL__" not in page and "__OPENROUTER_MODEL__" not in page


def test_page_renders_config_tiers_galleries():
    page = _render(candidates=["/t/a.png"],
                   config={"tiers": ["gold"], "galleries": ["Art"]})
    assert 'class="f-tier"' in page and ">gold<" in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    page = _render(candidates=["/t/a.png"], config=config,
                   schedules=schedule_data(config["schedule"]))
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page
    assert '"slots"' in page
    assert "__SCHEDULES__" not in page and "__EXISTING_TS__" not in page
    assert "__LABELS__" not in page


def test_pending_card_renders_with_its_schedule_and_uuid():
    pending = [{"uuid": "00000000-0000-4000-8000-000000000001", "path": "/t/a.png",
                "title": "Tickler", "description": "d", "scheduleTs": 1800000000,
                "price": None, "tier": None, "galleries": []}]
    page = _render(pending=pending, thumb_map={"/t/a.png": "/t/thumb_0000.png"})
    assert "Tickler" in page
    assert '00000000-0000-4000-8000-000000000001' in page   # PENDING payload injected
    assert "__PENDING__" not in page and "__CARDS__" not in page
