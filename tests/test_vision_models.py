from publicator.apps import vision_models


def test_main_reports_each_local_vision_model(monkeypatch, capsys):
    models = [
        {"ollama_name": "vision:fits"},
        {"ollama_name": "vision:too-large"},
    ]
    statuses = iter([
        {"fit": True, "fit_level": "Good", "reason": "Fits."},
        {"fit": False, "fit_level": "Too Tight", "reason": "Needs more memory."},
    ])
    monkeypatch.setattr(vision_models, "VISION_MODELS", models)
    monkeypatch.setattr(vision_models, "local_model_fit", lambda model: next(statuses))

    assert vision_models.main([]) == 0
    assert capsys.readouterr().out == (
        "vision:fits: fits (Good) — Fits.\n"
        "vision:too-large: does not fit (Too Tight) — Needs more memory.\n"
    )


def test_main_returns_failure_when_a_model_cannot_be_assessed(monkeypatch, capsys):
    monkeypatch.setattr(vision_models, "VISION_MODELS", [{"ollama_name": "vision:broken"}])
    monkeypatch.setattr(vision_models, "local_model_fit", lambda model: {
        "fit": False, "fit_level": "Unavailable", "reason": "llmfit failed"})

    assert vision_models.main([]) == 1
    assert "vision:broken: unavailable (Unavailable) — llmfit failed" in capsys.readouterr().out
