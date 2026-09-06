//! Evaluate independent cells in bounded batches; the coordinator alone owns
//! traversal, findings, error bounds and the global refinement budget.
use super::*;
use std::sync::mpsc::{Receiver, Sender, channel};

type Evaluated = Vec<(Cell, Result<Evaluation>)>;
type Work = Vec<(usize, Cell)>;
type Completed = Vec<(usize, Cell, Result<Evaluation>)>;
pub(super) struct Evaluator<'a> {
    ctx: &'a Context<'a>,
    sweeps: &'a [Sweep<'a>],
    options: &'a VerificationOptions,
    all_clear: bool,
    workers: Vec<(Sender<Work>, Receiver<Completed>)>,
}
impl<'a> Evaluator<'a> {
    pub fn new<'scope>(
        scope: &'scope std::thread::Scope<'scope, 'a>,
        ctx: &'a Context<'a>,
        sweeps: &'a [Sweep<'a>],
        options: &'a VerificationOptions,
        all_clear: bool,
        count: usize,
    ) -> Self
    where
        'a: 'scope,
    {
        let workers = (0..count)
            .map(|_| {
                let (send, receive) = channel::<Work>();
                let (completed, results) = channel();
                scope.spawn(move || {
                    while let Ok(cells) = receive.recv() {
                        let batch = cells
                            .into_iter()
                            .map(|(index, cell)| {
                                let result = evaluate(ctx, &cell, sweeps, options, all_clear);
                                (index, cell, result)
                            })
                            .collect();
                        if completed.send(batch).is_err() {
                            break;
                        }
                    }
                });
                (send, results)
            })
            .collect();
        Self {
            ctx,
            sweeps,
            options,
            all_clear,
            workers,
        }
    }
    pub fn batch(&self, cells: Vec<Cell>) -> Result<Evaluated> {
        if self.workers.is_empty() || cells.len() < self.workers.len() {
            return Ok(cells
                .into_iter()
                .map(|cell| {
                    let result =
                        evaluate(self.ctx, &cell, self.sweeps, self.options, self.all_clear);
                    (cell, result)
                })
                .collect());
        }
        let count = cells.len();
        let mut chunks: Vec<Work> = (0..self.workers.len()).map(|_| vec![]).collect();
        // Neighboring cells have very different costs near boundary/stock
        // transitions. Interleave them to avoid one slow spatial chunk.
        for (index, cell) in cells.into_iter().enumerate() {
            chunks[index % self.workers.len()].push((index, cell));
        }
        for ((send, _), chunk) in self.workers.iter().zip(chunks) {
            send.send(chunk)
                .map_err(|_| error("VERIFICATION_WORKER", "cell evaluator stopped"))?;
        }
        let mut evaluated: Vec<_> = (0..count).map(|_| None).collect();
        for (_, results) in &self.workers {
            for (index, cell, result) in results
                .recv()
                .map_err(|_| error("VERIFICATION_WORKER", "cell evaluator stopped"))?
            {
                evaluated[index] = Some((cell, result));
            }
        }
        Ok(evaluated
            .into_iter()
            .map(|entry| entry.expect("every dispatched cell returned"))
            .collect())
    }
}
