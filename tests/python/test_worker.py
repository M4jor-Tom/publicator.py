import io
import json
import os
import subprocess
import sys

from publicator_da import worker


def valid_commands(tmp_path):
    open_message = {"protocol": 1, "type": "open", "login_profile": str(tmp_path / ".deviantart-login"), "work_profile": str(tmp_path / ".deviantart-session"), "headless": False, "timezone": "Europe/Paris"}
    submit = {"protocol": 1, "type": "submit", "attempt_id": "00000000-0000-4000-8000-000000000999", "uuid": "00000000-0000-4000-8000-000000000001", "image_path": str(tmp_path / "upload.png"), "title": "Title", "description": "Story", "tags": ["art"], "schedule": {"epoch": 1800039600, "timezone": "Europe/Paris", "year": 2027, "month": 1, "day": 15, "hour": 20, "minute": 0}, "price": 0, "tiers": ["Gold"], "galleries": ["Featured"]}
    close = {"protocol": 1, "type": "close"}
    return "\n".join(map(json.dumps, (open_message, submit, close))) + "\n"


class FakeSession:
    def __init__(self, open_command, data_root):
        self.open_command = open_command
        self.data_root = data_root
        self.closed = False

    def submit(self, job):
        return {"protocol": 1, "type": "scheduled", "attempt_id": job["attempt_id"], "uuid": job["uuid"]}

    def close(self):
        self.closed = True


def test_worker_rejects_foreign_profile_pair_before_session_acquisition(tmp_path):
    opened = []

    class Session(FakeSession):
        def __init__(self, command, data_root):
            opened.append((command, data_root))

    command = {
        "protocol": 1, "type": "open",
        "login_profile": str(tmp_path / ".deviantart-login"),
        "work_profile": str(tmp_path / "unrelated"),
        "headless": True, "timezone": "Europe/Paris",
    }
    output = io.StringIO()
    assert worker.serve(io.StringIO(json.dumps(command) + "\n"), output, tmp_path, Session) != 0
    assert opened == []
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["fatal"]


def test_worker_refuses_same_name_foreign_root_and_forged_open_root(tmp_path):
    trusted = tmp_path / "trusted"
    trusted.mkdir()
    foreign = tmp_path / "foreign"
    source = foreign / ".deviantart-login"
    work = foreign / ".deviantart-session"
    source.mkdir(parents=True)
    work.mkdir()
    sentinel = work / "sentinel"
    sentinel.write_text("keep")
    opened = []

    class Session(FakeSession):
        def __init__(self, command, data_root):
            opened.append((command, data_root))

    command = {
        "protocol": 1, "type": "open",
        "login_profile": str(source), "work_profile": str(work),
        "headless": True, "timezone": "Europe/Paris",
    }
    output = io.StringIO()
    assert worker.serve(io.StringIO(json.dumps(command) + "\n"), output, trusted, Session) != 0
    assert opened == []
    assert sentinel.read_text() == "keep"

    command["data_root"] = str(foreign)
    output = io.StringIO()
    assert worker.serve(io.StringIO(json.dumps(command) + "\n"), output, trusted, Session) != 0
    assert opened == []
    assert sentinel.read_text() == "keep"


def test_worker_imports_no_application_modules():
    import ast
    source = __import__("pathlib").Path(worker.__file__).read_text()
    imports = [node.module or "" for node in ast.walk(ast.parse(source)) if isinstance(node, ast.ImportFrom)]
    assert not any(name.startswith("publicator.apps") or name in {"publicator.store", "publicator.config", "publicator.entries", "publicator.workflow"} for name in imports)


def test_worker_never_reads_publication_database(tmp_path, monkeypatch):
    forbidden = {"publications.json", "publicator.toml"}
    original = open

    def guarded(path, *args, **kwargs):
        assert str(path).rsplit("/", 1)[-1] not in forbidden
        return original(path, *args, **kwargs)

    monkeypatch.setattr("builtins.open", guarded)
    output = io.StringIO()
    assert worker.serve(io.StringIO(valid_commands(tmp_path)), output, tmp_path, FakeSession) == 0
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "scheduled"]


def test_worker_main_binds_data_root_from_startup_argument(tmp_path, monkeypatch):
    captured = []

    class Session(FakeSession):
        def __init__(self, command, data_root):
            captured.append(data_root)

    monkeypatch.setattr(worker, "BrowserSession", Session)
    monkeypatch.setattr(worker.sys, "argv", ["publicator-da-worker", "--data-root", str(tmp_path)])
    monkeypatch.setattr(worker.sys, "stdin", io.StringIO(valid_commands(tmp_path)))
    output = io.StringIO()
    monkeypatch.setattr(worker.sys, "stdout", output)

    assert worker.main() == 0
    assert captured == [tmp_path.resolve()]
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "scheduled"]


def test_worker_subprocess_preserves_json_line_framing(tmp_path):
    script = tmp_path / "fake_worker.py"
    script.write_text('''import sys\nfrom pathlib import Path\nfrom publicator_da.worker import serve\nclass Fake:\n def __init__(self, cmd, root): pass\n def submit(self, job): return {"type":"scheduled", "attempt_id":job["attempt_id"], "uuid":job["uuid"]}\n def close(self): pass\nraise SystemExit(serve(sys.stdin, sys.stdout, Path(sys.argv[2]), Fake))\n''')
    env = dict(os.environ, PYTHONPATH=str(__import__("pathlib").Path(__file__).parents[2] / "python"))
    result = subprocess.run([sys.executable, str(script), "--data-root", str(tmp_path)], input=valid_commands(tmp_path), text=True, capture_output=True, env=env, check=False)
    assert result.returncode == 0
    assert [json.loads(line)["type"] for line in result.stdout.splitlines()] == ["ready", "scheduled"]
    assert result.stderr == ""


def test_worker_stops_after_terminal_submission_error(tmp_path):
    commands = valid_commands(tmp_path).splitlines()
    submit = json.loads(commands[1])
    commands.insert(2, json.dumps(submit))
    class Failed(FakeSession):
        def submit(self, job):
            return {"protocol": 1, "type": "unknown", "attempt_id": job["attempt_id"], "uuid": job["uuid"], "error": "clicked"}
    output = io.StringIO()
    assert worker.serve(io.StringIO("\n".join(commands) + "\n"), output, tmp_path, Failed) == 1
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "unknown"]


def test_worker_rejects_bad_submit_before_session_mutation(tmp_path):
    commands = json.loads(valid_commands(tmp_path).splitlines()[1])
    commands["schedule"]["minute"] = 1
    touched = []
    class Session(FakeSession):
        def submit(self, job):
            touched.append(job)
            return super().submit(job)
    output = io.StringIO()
    assert worker.serve(io.StringIO(json.dumps({"protocol": 1, "type": "open", "login_profile": str(tmp_path / ".deviantart-login"), "work_profile": str(tmp_path / ".deviantart-session"), "headless": False, "timezone": "Europe/Paris"}) + "\n" + json.dumps(commands) + "\n"), output, tmp_path, Session) != 0
    assert not touched
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "fatal"]
