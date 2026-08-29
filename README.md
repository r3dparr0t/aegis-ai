# Aegis-AI 🛡️🤖

**Aegis-AI** is a lightweight, high-performance, AI-assisted Fuzzing and Adaptive Execution Engine written in Rust. It utilizes Small Language Models (SLMs) via **Ollama** alongside an episodic memory layer to iteratively refine model outputs, evaluate edge cases, and learn from execution failures with minimal iteration cycles.

---

## ✨ Features

- **Adaptive Reflexion Loop:** Learns from execution errors in real-time without computationally expensive full-model fine-tuning.
- **Hybrid Memory Engine:** Powered by SQLite with multi-attribute filtering (Task Metadata + Error Taxonomy + Keyword Indexing) for context retrieval.
- **Decoupled Architecture:** Strict separation between LLM Transport (`LlmProvider`), Evaluation Logic (`Evaluator`), and State Management.
- **Asynchronous & Fast:** Built natively with Rust's async ecosystem for minimal overhead and zero garbage collection pauses.

---

## 🚀 Quick Start

### 1. Prerequisites

Make sure you have [Rust](https://www.rust-lang.org/) and [Ollama](https://ollama.com/) installed on your machine.

### 2. Setup LLM Environment

Pull and start the recommended model (`qwen2.5:3b`):

```bash
ollama run qwen2.5:3b
