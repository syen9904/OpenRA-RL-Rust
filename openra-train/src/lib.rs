//! Training runtime for RL agents.
//!
//! Manages 128+ parallel game simulations in a single process.
//! Exposes a Python API via PyO3 for direct integration with
//! training pipelines (TRL GRPOTrainer, etc.).
