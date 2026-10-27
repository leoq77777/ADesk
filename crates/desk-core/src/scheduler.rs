use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub run_id: String,
    pub session_id: String,
    pub seq: i64,
    pub manual: bool,
}

pub struct Scheduler {
    max_global: usize,
    next_seq: i64,
    active: HashMap<String, String>,
    queue: Vec<Job>,
}

impl Scheduler {
    pub fn new(max_global: usize) -> Self {
        Self {
            max_global: max_global.max(1),
            next_seq: 0,
            active: HashMap::new(),
            queue: Vec::new(),
        }
    }

    pub fn enqueue(&mut self, session_id: &str, run_id: &str, manual: bool) -> Job {
        self.push(session_id, run_id, manual, self.next_seq)
    }

    pub fn enqueue_front(&mut self, session_id: &str, run_id: &str) -> Job {
        let seq = self
            .queue
            .iter()
            .filter(|job| job.session_id == session_id)
            .map(|job| job.seq)
            .min()
            .unwrap_or(self.next_seq)
            - 1;
        self.push(session_id, run_id, true, seq)
    }

    fn push(&mut self, session_id: &str, run_id: &str, manual: bool, seq: i64) -> Job {
        let job = Job {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
            seq,
            manual,
        };
        if seq >= self.next_seq {
            self.next_seq = seq + 1;
        }
        self.queue.push(job.clone());
        job
    }

    pub fn active_run(&self, session_id: &str) -> Option<String> {
        self.active.get(session_id).cloned()
    }

    pub fn dispatch_ready(&mut self) -> Vec<Job> {
        let mut started = Vec::new();
        while self.active.len() < self.max_global {
            let Some(idx) = self.pick_index() else { break };
            let job = self.queue.remove(idx);
            self.active.insert(job.session_id.clone(), job.run_id.clone());
            started.push(job);
        }
        started
    }

    pub fn finish(&mut self, session_id: &str, run_id: &str) {
        if self.active.get(session_id).map(String::as_str) == Some(run_id) {
            self.active.remove(session_id);
        }
    }

    pub fn cancel_queued(&mut self, run_id: &str) -> bool {
        let before = self.queue.len();
        self.queue.retain(|job| job.run_id != run_id);
        before != self.queue.len()
    }

    pub fn clear_session(&mut self, session_id: &str) -> Option<String> {
        self.queue.retain(|job| job.session_id != session_id);
        self.active.remove(session_id)
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn is_active(&self, session_id: &str) -> bool {
        self.active.contains_key(session_id)
    }

    pub fn queued_run_ids(&self) -> Vec<String> {
        self.queue.iter().map(|job| job.run_id.clone()).collect()
    }

    pub fn queued_count_for(&self, session_id: &str) -> usize {
        self.queue
            .iter()
            .filter(|job| job.session_id == session_id)
            .count()
    }

    fn pick_index(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, job) in self.queue.iter().enumerate() {
            if self.active.contains_key(&job.session_id) {
                continue;
            }
            if self
                .queue
                .iter()
                .any(|other| other.session_id == job.session_id && other.seq < job.seq)
            {
                continue;
            }
            best = Some(match best {
                None => i,
                Some(prev) => {
                    let other = &self.queue[prev];
                    if job.manual && !other.manual {
                        i
                    } else if job.manual == other.manual && job.seq < other.seq {
                        i
                    } else {
                        prev
                    }
                }
            });
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u09_same_session_serial_cross_session_respects_cap() {
        let mut s = Scheduler::new(2);
        s.enqueue("a", "a1", true);
        s.enqueue("a", "a2", true);
        s.enqueue("b", "b1", true);
        s.enqueue("c", "c1", true);
        let started = s.dispatch_ready();
        let ids: Vec<_> = started.iter().map(|j| j.run_id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1"]);
        assert_eq!(s.queued_run_ids(), vec!["a2".to_string(), "c1".to_string()]);

        let mut one = Scheduler::new(1);
        one.enqueue("a", "a1", true);
        one.enqueue("b", "b1", true);
        let started = one.dispatch_ready();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].run_id, "a1");
    }

    #[test]
    fn u10_manual_first_no_reorder_inside_session() {
        let mut s = Scheduler::new(1);
        s.enqueue("a", "a-auto", false);
        s.enqueue("a", "a-manual", true);
        s.enqueue("b", "b-manual", true);
        let first = s.dispatch_ready();
        assert_eq!(first[0].run_id, "b-manual");
        s.finish("b", "b-manual");
        let second = s.dispatch_ready();
        assert_eq!(second[0].run_id, "a-auto");
    }

    #[test]
    fn u11_cancel_queued_and_finish_is_idempotent() {
        let mut s = Scheduler::new(1);
        s.enqueue("a", "a1", true);
        s.enqueue("a", "a2", true);
        s.dispatch_ready();
        assert!(s.cancel_queued("a2"));
        assert!(s.queued_run_ids().is_empty());
        s.finish("a", "a1");
        s.finish("a", "a1");
        assert_eq!(s.active_count(), 0);
        assert!(s.dispatch_ready().is_empty());
    }

    #[test]
    fn stop_and_replace_goes_before_other_queued() {
        let mut s = Scheduler::new(1);
        s.enqueue("a", "a1", true);
        s.enqueue("a", "a2", true);
        s.dispatch_ready();
        s.enqueue_front("a", "a-new");
        s.finish("a", "a1");
        let next = s.dispatch_ready();
        assert_eq!(next[0].run_id, "a-new");
        assert_eq!(s.queued_run_ids(), vec!["a2".to_string()]);
    }
}
