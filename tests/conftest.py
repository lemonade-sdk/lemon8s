"""Fixtures for the end-to-end tests, which run against a live lemon8s install."""

import dataclasses
import json
import os
import urllib.error
import urllib.parse
import urllib.request

import pytest


class Lemonade:
    """Lemonade's API, as a client of the install reaches it."""

    def __init__(self, base_url: str, key: str) -> None:
        self.base_url = base_url.rstrip("/") + "/"
        self.key = key

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
            path: Relative to the base URL, such as `health`, or from the server's
                root, such as `/internal/config`.
            body: The JSON body of a POST, or `None` for a GET.
            authorized: Whether to send the API key.
            timeout: Seconds to wait for the answer.

        Returns:
            The HTTP status and the decoded body.
        """
        headers = {"Content-Type": "application/json"}
        if authorized:
            headers["Authorization"] = f"Bearer {self.key}"
        request = urllib.request.Request(
            urllib.parse.urljoin(self.base_url, path),
            data=None if body is None else json.dumps(body).encode(),
            headers=headers,
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            return error.code, json.loads(error.read() or b"{}")

    def load(self, name: str) -> dict:
        """Pulls and loads a model, as an app's first request would.

        Args:
            name: A model in Lemonade's registry.

        Returns:
            The model's entry in `health`, which says where it runs.
        """
        status, body = self.call("pull", {"model_name": name}, timeout=1800)
        assert status == 200 and body.get("status") == "success", body
        status, body = self.call("load", {"model_name": name}, timeout=1800)
        assert status == 200, body
        status, health = self.call("health")
        assert status == 200, health
        entries = [m for m in health["all_models_loaded"] if m["model_name"] == name]
        assert entries, f"health doesn't list {name} as loaded: {health}"
        return entries[0]

    def ask(self, model: str, question: str, *, timeout: float = 60) -> str:
        """Asks a model one question, without sampling or thinking.

        Args:
            model: A loaded model.
            question: The user's message.
            timeout: Seconds to wait for the answer.

        Returns:
            The model's answer.
        """
        status, chat = self.call(
            "chat/completions",
            {
                "model": model,
                "messages": [{"role": "user", "content": question}],
                "temperature": 0,
                "max_tokens": 32,
                "enable_thinking": False,
            },
            timeout=timeout,
        )
        assert status == 200, chat
        return chat["choices"][0]["message"]["content"]

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
    parser.addoption(
        "--npu-model",
        help="an NPU model in Lemonade's registry, such as qwen3-0.6b-FLM, to test the NPU",
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
    entry = lemonade.load(name)
    after = lemonade.gpu_gb_used()
    return LoadedModel(
        entry=entry,
        gpu_gb_gained=None if before is None else after - before,
    )
