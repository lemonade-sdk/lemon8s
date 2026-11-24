"""Lemonade's other engines on a lemon8s install: embeddings, reranking, speech, and images."""

import base64
import io
import json
import math
import re
import struct
import wave

import pytest

SPOKEN = "The quick brown fox jumps over the lazy dog."
# Seconds for a first request, which may start an engine and download its model.
FIRST_USE = 600


@pytest.fixture(scope="module")
def spoken(lemonade) -> bytes:
    """Kokoro reading SPOKEN aloud, as a 16-bit WAV that every speech-to-text engine reads."""
    status, headers, pcm = lemonade.send(
        "audio/speech",
        json.dumps(
            {
                "model": "kokoro-v1",
                "input": SPOKEN,
                "voice": "af_sky",
                "response_format": "pcm",
            }
        ).encode(),
        timeout=FIRST_USE,
    )
    assert status == 200, pcm[:300]
    assert headers.get_content_type() == "audio/l16", headers
    wav = io.BytesIO()
    with wave.open(wav, "wb") as writer:
        writer.setnchannels(1)
        writer.setsampwidth(2)
        writer.setframerate(int(headers.get_param("rate")))
        writer.writeframes(pcm)
    return wav.getvalue()


def test_similar_sentences_embed_closer_than_unrelated_ones(lemonade):
    status, body = lemonade.call(
        "embeddings",
        {
            "model": "nomic-embed-text-v1-GGUF",
            "input": [
                "A cat sits on the mat.",
                "A kitten is sitting on a rug.",
                "Quarterly revenue rose by ten percent.",
            ],
        },
        timeout=FIRST_USE,
    )
    assert status == 200, body
    cat, kitten, revenue = (item["embedding"] for item in body["data"])
    assert math.dist(cat, kitten) < math.dist(cat, revenue)


def test_the_reranker_puts_the_relevant_document_first(lemonade):
    status, body = lemonade.call(
        "rerank",
        {
            "model": "jina-reranker-v1-tiny-en-GGUF",
            "query": "What do cats like to do?",
            "documents": [
                "The stock market fell today.",
                "Cats like to chase mice and sleep in the sun.",
            ],
        },
        timeout=FIRST_USE,
    )
    assert status == 200, body
    best = max(body["results"], key=lambda result: result["relevance_score"])
    assert best["index"] == 1, body


def test_text_to_speech_reads_the_sentence_aloud(spoken):
    with wave.open(io.BytesIO(spoken)) as reader:
        seconds = reader.getnframes() / reader.getframerate()
    assert 1.5 < seconds < 10, seconds


@pytest.mark.parametrize("model", ["Whisper-Tiny", "Moonshine-Tiny-Streaming"])
def test_speech_to_text_writes_down_what_was_said(lemonade, spoken, model):
    status, body = lemonade.transcribe(model, spoken, timeout=FIRST_USE)
    assert status == 200, body
    words = re.sub(r"[^a-z ]", "", body["text"].lower())
    assert "quick brown fox" in words and "lazy dog" in words, body


@pytest.mark.slow
def test_stable_diffusion_draws_an_image_of_the_requested_size(lemonade):
    status, body = lemonade.call(
        "images/generations",
        {
            "model": "SD-Turbo-GGUF",
            "prompt": "a red apple on a white table",
            "size": "256x256",
            "steps": 1,
            "response_format": "b64_json",
        },
        timeout=1800,
    )
    assert status == 200, body
    png = base64.b64decode(body["data"][0]["b64_json"])
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    assert struct.unpack(">II", png[16:24]) == (256, 256)
