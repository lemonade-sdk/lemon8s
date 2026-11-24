"""Lemonade on a lemon8s install: the keys, the backend, the GPU, and an answer."""

import pytest

MIN_GPU_GB = 0.25


def test_a_request_without_the_api_key_is_refused(lemonade):
    status, body = lemonade.call("models", authorized=False)
    assert status == 401, body


def test_the_api_key_cannot_reach_lemonade_settings(lemonade):
    status, body = lemonade.call("/internal/config")
    assert status == 401, body


def test_the_model_runs_with_the_chosen_backend(loaded_model, backend):
    binary = loaded_model.entry["launch_command"][0]
    assert f"/llamacpp/{backend}" in binary, binary


def test_a_model_on_the_gpu_takes_gpu_memory(loaded_model):
    device = loaded_model.entry["device"]
    if device != "gpu":
        pytest.skip(f"the model runs on the {device}")
    gained = loaded_model.gpu_gb_gained
    assert gained is not None, "Lemonade sees no GPU"
    assert gained >= MIN_GPU_GB, f"GPU memory in use rose by {gained:.2f} GB"


def test_the_model_answers_a_simple_question(lemonade, loaded_model):
    answer = lemonade.ask(
        loaded_model.entry["model_name"],
        "What is 7 times 6? Answer with the number only.",
    )
    assert "42" in answer, answer
