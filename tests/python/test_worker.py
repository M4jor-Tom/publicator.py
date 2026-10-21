import io
import json
import os
import subprocess
import sys

from publicator_da import worker


def valid_commands(tmp_path):
    open_message = {"protocol": 1, "type": "open", "login_profile": str(tmp_path / "login"), "work_profile": str(tmp_path / "work"), "headless": False, "timezone": "Europe/Paris"}
    submit = {"protocol": 1, "type": "submit", "attempt_id": "00000000-0000-4000-8000-000000000999", "uuid": "00000000-0000-4000-8000-000000000001", "image_path": str(tmp_path / "upload.png"), "title": "Title", "description": "Story", "tags": ["art"], "schedule": {"epoch": 1800039600, "timezone": "Europe/Paris", "year": 2027, "month": 1, "day": 15, "hour": 20, "minute": 0}, "price": 0, "tiers": ["Gold"], "galleries": ["Featured"]}
    close = {"protocol": 1, "type": "close"}
    return "\n".join(map(json.dumps, (open_message, submit, close))) + "\n"


class FakeSession:
    def __init__(self, open_command):
        self.open_command = open_command
        self.closed = False

    def submit(self, job):
        return {"protocol": 1, "type": "scheduled", "attempt_id": job["attempt_id"], "uuid": job["uuid"]}

    def close(self):
        self.closed = True


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
    assert worker.serve(io.StringIO(valid_commands(tmp_path)), output, FakeSession) == 0
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "scheduled"]


def test_worker_subprocess_preserves_json_line_framing(tmp_path):
    script = tmp_path / "fake_worker.py"
    script.write_text('''import sys\nfrom publicator_da.worker import serve\nclass Fake:\n def __init__(self, cmd): pass\n def submit(self, job): return {"type":"scheduled", "attempt_id":job["attempt_id"], "uuid":job["uuid"]}\n def close(self): pass\nraise SystemExit(serve(sys.stdin, sys.stdout, Fake))\n''')
    env = dict(os.environ, PYTHONPATH=str(__import__("pathlib").Path(__file__).parents[2] / "python"))
    result = subprocess.run([sys.executable, str(script)], input=valid_commands(tmp_path), text=True, capture_output=True, env=env, check=False)
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
    assert worker.serve(io.StringIO("\n".join(commands) + "\n"), output, Failed) == 1
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
    assert worker.serve(io.StringIO(json.dumps({"protocol": 1, "type": "open", "login_profile": str(tmp_path / "login"), "work_profile": str(tmp_path / "work"), "headless": False, "timezone": "Europe/Paris"}) + "\n" + json.dumps(commands) + "\n"), output, Session) != 0
    assert not touched
    assert [json.loads(line)["type"] for line in output.getvalue().splitlines()] == ["ready", "fatal"]
