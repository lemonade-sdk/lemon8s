"""Lemonade on a lemon8s install with an NPU: a model runs on the NPU and answers."""

import pytest


def test_a_model_on_the_npu_answers_a_simple_question(lemonade, pytestconfig):
    name = pytestconfig.getoption("--npu-model")
    if name is None:
        pytest.skip("give --npu-model to test the NPU")
    status, body = lemonade.call(
        "install", {"recipe": "flm", "backend": "npu"}, timeout=1800
    )
    assert status == 200, body
    entry = lemonade.load(name)
    assert entry["device"] == "npu", entry
    answer = lemonade.ask(
        name, "What is 7 times 6? Answer with the number only.", timeout=600
    )
    assert "42" in answer, answer
