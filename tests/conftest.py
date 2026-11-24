"""Fixtures for the end-to-end tests, which run against a live lemon8s install."""

import dataclasses
import email.message
import json
import os
import urllib.error
import urllib.parse
import urllib.request
import uuid

import pytest


class Lemonade:
    """Lemonade's API, as a client of the install reaches it."""

    def __init__(self, base_url: str, key: str) -> None:
        self.base_url = base_url.rstrip("/") + "/"
        self.key = key

    def send(
        self,
        path: str,
        data: bytes | None = None,
        content_type: str = "application/json",
        *,
        authorized: bool = True,
        timeout: float = 60,
    ) -> tuple[int, email.message.Message, bytes]:
        """Sends one request and returns its answer as it came.

        Args:
            path: Relative to the base URL, such as `health`, or from the server's
                root, such as `/internal/config`.
            data: The body of a POST, or `None` for a GET.
            content_type: The body's media type.
            authorized: Whether to send the API key.
            timeout: Seconds to wait for the answer.

        Returns:
            The HTTP status, the answer's headers, and its bytes.
        """
        headers = {"Content-Type": content_type}
        if authorized:
            headers["Authorization"] = f"Bearer {self.key}"
        request = urllib.request.Request(
            urllib.parse.urljoin(self.base_url, path), data=data, headers=headers
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.status, response.headers, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.headers, error.read()

    def call(
        self,
        path: str,
        body: dict | None = None,
        *,
        authorized: bool = True,
        timeout: float = 60,
    ) -> tuple[int, dict]:
        """Sends one request and decodes its JSON answer.

        Args:
            path: As for `send`.
            body: The JSON body of a POST, or `None` for a GET.
            authorized: Whether to send the API key.
            timeout: Seconds to wait for the answer.

        Returns:
            The HTTP status and the decoded body.
        """
        status, _, raw = self.send(
            path,
            None if body is None else json.dumps(body).encode(),
            authorized=authorized,
            timeout=timeout,
        )
        return status, json.loads(raw or b"{}")

    def transcribe(
        self, model: str, wav: bytes, *, timeout: float = 60
    ) -> tuple[int, dict]:
        """Uploads a WAV file for transcription, as a form with the model's name.

        Args:
            model: A speech-to-text model in Lemonade's registry.
            wav: The audio to transcribe.
            timeout: Seconds to wait for the answer.

        Returns:
            The HTTP status and the decoded body.
        """
        boundary = uuid.uuid4().hex
        form = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="model"\r\n\r\n{model}\r\n'
            f"--{boundary}\r\n"
            'Content-Disposition: form-data; name="file"; filename="speech.wav"\r\n'
            "Content-Type: audio/wav\r\n\r\n"
        ).encode()
        status, _, raw = self.send(
            "audio/transcriptions",
            form + wav + f"\r\n--{boundary}--\r\n".encode(),
            f"multipart/form-data; boundary={boundary}",
            timeout=timeout,
        )
        return status, json.loads(raw or b"{}")

    def gpu_gb_used(self) -> float | None:
        """Sums the memory in use on every GPU Lemonade sees.

        Returns:
            Gigabytes in use, or `None` if Lemonade sees no GPU.

        Raises:
            AssertionError: Lemonade sees a GPU but doesn't report its memory.
        """
        status, stats = self.call("system-stats")
        assert status == 200, stats
        used = [gpu["memory_used_gb"] for gpu in stats["gpus"]]
        assert None not in used, f"GPU memory not reported: {stats}"
        return sum(used) if used else None


@dataclasses.dataclass(frozen=True)
class LoadedModel:
    """A model the tests pulled and loaded, and the GPU memory loading it took."""

    entry: dict
    gpu_gb_gained: float | None


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--backend", help="the llama.cpp backend the install runs, such as vulkan"
    )
    parser.addoption(
        "--model", default="Qwen3-0.6B-GGUF", help="a model in Lemonade's registry"
    )


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line(
        "markers", "slow: too slow or too large a download for CI, which skips it"
    )


@pytest.fixture(scope="session")
def backend(pytestconfig: pytest.Config) -> str:
    chosen = pytestconfig.getoption("--backend")
    if chosen is None:
        raise pytest.UsageError("--backend is required: the backend the install runs")
    return chosen


@pytest.fixture(scope="session")
def lemonade() -> Lemonade:
    base_url = os.environ.get("OPENAI_BASE_URL")
    key = os.environ.get("OPENAI_API_KEY")
    if not base_url or not key:
        raise pytest.UsageError(
            "set OPENAI_BASE_URL and OPENAI_API_KEY as the README does"
        )
    return Lemonade(base_url, key)


@pytest.fixture(scope="session")
def loaded_model(pytestconfig: pytest.Config, lemonade: Lemonade) -> LoadedModel:
    name = pytestconfig.getoption("--model")
    status, body = lemonade.call("unload", {})
    assert status == 200, body
    before = lemonade.gpu_gb_used()
    status, body = lemonade.call("pull", {"model_name": name}, timeout=1800)
    assert status == 200 and body.get("status") == "success", body
    status, body = lemonade.call("load", {"model_name": name}, timeout=1800)
    assert status == 200, body
    after = lemonade.gpu_gb_used()
    status, health = lemonade.call("health")
    assert status == 200, health
    entries = [m for m in health["all_models_loaded"] if m["model_name"] == name]
    assert entries, f"health doesn't list {name} as loaded: {health}"
    return LoadedModel(
        entry=entries[0],
        gpu_gb_gained=None if before is None else after - before,
    )
