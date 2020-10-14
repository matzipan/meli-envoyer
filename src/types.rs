/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

/*! UI types used throughout meli.
 *
 * The `segment_tree` module performs maximum range queries. This is used in getting the maximum
 * element of a column within a specific range in e-mail lists. That way a very large value that
 * is not the in the currently displayed page does not cause the column to be rendered bigger
 * than it has to.
 *
 * `UIMode` describes the application's... mode. Same as in the modal editor `vi`.
 *
 * `UIEvent` is the type passed around `Component`s when something happens.
 */
extern crate serde;
#[macro_use]
mod helpers;
pub use self::helpers::*;

use super::command::Action;
use super::jobs::JobId;
use super::terminal::*;
use crate::components::{Component, ComponentId};

use melib::backends::{AccountHash, BackendEvent, MailboxHash};
use melib::{EnvelopeHash, RefreshEvent, ThreadHash};
use nix::unistd::Pid;
use std::fmt;
use uuid::Uuid;

#[derive(Debug)]
pub enum StatusEvent {
    DisplayMessage(String),
    BufClear,
    BufSet(String),
    UpdateStatus(String),
    NewJob(JobId),
    JobFinished(JobId),
    JobCanceled(JobId),
    SetMouse(bool),
}

/// `ThreadEvent` encapsulates all of the possible values we need to transfer between our threads
/// to the main process.
#[derive(Debug)]
pub enum ThreadEvent {
    /// User input.
    Input((Key, Vec<u8>)),
    /// User input and input as raw bytes.
    /// A watched Mailbox has been refreshed.
    RefreshMailbox(Box<RefreshEvent>),
    UIEvent(UIEvent),
    /// A thread has updated some of its information
    Pulse,
    //Decode { _ }, // For gpg2 signature check
    JobFinished(JobId),
}

impl From<RefreshEvent> for ThreadEvent {
    fn from(event: RefreshEvent) -> Self {
        ThreadEvent::RefreshMailbox(Box::new(event))
    }
}

#[derive(Debug)]
pub enum ForkType {
    /// Already finished fork, we only want to restore input/output
    Finished,
    /// Embed pty
    Embed(Pid),
    Generic(std::process::Child),
    NewDraft(File, std::process::Child),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum NotificationType {
    Info,
    Error(melib::error::ErrorKind),
    NewMail,
    SentMail,
    Saved,
}

impl core::fmt::Display for NotificationType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            NotificationType::Info => write!(f, "info"),
            NotificationType::Error(melib::error::ErrorKind::None) => write!(f, "error"),
            NotificationType::Error(kind) => write!(f, "error: {}", kind),
            NotificationType::NewMail => write!(f, "new mail"),
            NotificationType::SentMail => write!(f, "sent mail"),
            NotificationType::Saved => write!(f, "saved"),
        }
    }
}

#[derive(Debug)]
pub enum UIEvent {
    Input(Key),
    CmdInput(Key),
    InsertInput(Key),
    EmbedInput((Key, Vec<u8>)),
    //Quit?
    Resize,
    /// Force redraw.
    Fork(ForkType),
    ChangeMailbox(usize),
    ChangeMode(UIMode),
    Command(String),
    Notification(Option<String>, String, Option<NotificationType>),
    Action(Action),
    StatusEvent(StatusEvent),
    MailboxUpdate((AccountHash, MailboxHash)), // (account_idx, mailbox_idx)
    MailboxDelete((AccountHash, MailboxHash)),
    MailboxCreate((AccountHash, MailboxHash)),
    AccountStatusChange(AccountHash),
    ComponentKill(Uuid),
    BackendEvent(AccountHash, BackendEvent),
    StartupCheck(MailboxHash),
    RefreshEvent(Box<RefreshEvent>),
    EnvelopeUpdate(EnvelopeHash),
    EnvelopeRename(EnvelopeHash, EnvelopeHash), // old_hash, new_hash
    EnvelopeRemove(EnvelopeHash, ThreadHash),
    Contacts(ContactEvent),
    Compose(ComposeEvent),
    FinishedUIDialog(ComponentId, UIMessage),
    Callback(CallbackFn),
    GlobalUIDialog(Box<dyn Component>),
    Timer(u8),
}

pub struct CallbackFn(pub Box<dyn FnOnce(&mut crate::Context) -> () + Send + 'static>);

impl core::fmt::Debug for CallbackFn {
    fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(fmt, "CallbackFn")
    }
}

impl From<RefreshEvent> for UIEvent {
    fn from(event: RefreshEvent) -> Self {
        UIEvent::RefreshEvent(Box::new(event))
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum UIMode {
    Normal,
    Insert,
    /// Forward input to an embed pseudoterminal.
    Embed,
    Command,
    Fork,
}

impl fmt::Display for UIMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match *self {
                UIMode::Normal => "NORMAL",
                UIMode::Insert => "INSERT",
                UIMode::Command => "COMMAND",
                UIMode::Fork => "FORK",
                UIMode::Embed => "EMBED",
            }
        )
    }
}

/// An event notification that is passed to Entities for handling.
pub struct Notification {
    _title: String,
    _content: String,

    _timestamp: std::time::Instant,
}

pub mod segment_tree {
    /*! Simple segment tree implementation for maximum in range queries. This is useful if given an
     *  array of numbers you want to get the maximum value inside an interval quickly.
     */
    use smallvec::SmallVec;
    use std::convert::TryFrom;
    use std::iter::FromIterator;

    #[derive(Default, Debug, Clone)]
    pub struct SegmentTree {
        pub array: SmallVec<[u8; 1024]>,
        tree: SmallVec<[u8; 1024]>,
    }

    impl From<SmallVec<[u8; 1024]>> for SegmentTree {
        fn from(val: SmallVec<[u8; 1024]>) -> SegmentTree {
            SegmentTree::new(val)
        }
    }

    impl SegmentTree {
        pub fn new(val: SmallVec<[u8; 1024]>) -> SegmentTree {
            if val.is_empty() {
                return SegmentTree {
                    array: val.clone(),
                    tree: val,
                };
            }

            let height = (f64::from(u32::try_from(val.len()).unwrap_or(0)))
                .log2()
                .ceil() as u32;
            let max_size = 2 * (2_usize.pow(height));

            let mut segment_tree: SmallVec<[u8; 1024]> =
                SmallVec::from_iter(core::iter::repeat(0).take(max_size));
            for i in 0..val.len() {
                segment_tree[val.len() + i] = val[i];
            }

            for i in (1..val.len()).rev() {
                segment_tree[i] = std::cmp::max(segment_tree[2 * i], segment_tree[2 * i + 1]);
            }

            SegmentTree {
                array: val,
                tree: segment_tree,
            }
        }

        /// (left, right) is inclusive
        pub fn get_max(&self, mut left: usize, mut right: usize) -> u8 {
            if self.array.is_empty() {
                return 0;
            }

            let len = self.array.len();
            debug_assert!(left <= right);
            if right >= len {
                right = len.saturating_sub(1);
            }

            left += len;
            right += len + 1;

            let mut max = 0;

            while left < right {
                if (left & 1) > 0 {
                    max = std::cmp::max(max, self.tree[left]);
                    left += 1;
                }

                if (right & 1) > 0 {
                    right -= 1;
                    max = std::cmp::max(max, self.tree[right]);
                }

                left /= 2;
                right /= 2;
            }
            max
        }

        pub fn update(&mut self, pos: usize, value: u8) {
            let mut ctr = pos + self.array.len();

            // Update leaf node value
            self.tree[ctr] = value;
            while ctr > 1 {
                // move up one level
                ctr >>= 1;

                self.tree[ctr] = std::cmp::max(self.tree[2 * ctr], self.tree[2 * ctr + 1]);
            }
        }
    }

    #[test]
    fn test_segment_tree() {
        let array: SmallVec<[u8; 1024]> = [9, 1, 17, 2, 3, 23, 4, 5, 6, 37]
            .iter()
            .cloned()
            .collect::<SmallVec<[u8; 1024]>>();
        let mut segment_tree = SegmentTree::from(array.clone());

        assert_eq!(segment_tree.get_max(0, 5), 23);
        assert_eq!(segment_tree.get_max(6, 9), 37);

        segment_tree.update(2_usize, 24_u8);

        assert_eq!(segment_tree.get_max(0, 5), 24);
    }
}

#[derive(Debug)]
pub struct RateLimit {
    last_tick: std::time::Instant,
    pub timer: crate::timer::PosixTimer,
    rate: std::time::Duration,
    reqs: u64,
    millis: std::time::Duration,
    pub active: bool,
}

//FIXME: tests.
impl RateLimit {
    pub fn new(reqs: u64, millis: u64) -> Self {
        RateLimit {
            last_tick: std::time::Instant::now(),
            timer: crate::timer::PosixTimer::new_with_signal(
                std::time::Duration::from_secs(0),
                std::time::Duration::from_millis(millis),
                nix::sys::signal::Signal::SIGALRM,
            )
            .unwrap(),

            rate: std::time::Duration::from_millis(millis / reqs),
            reqs,
            millis: std::time::Duration::from_millis(millis),
            active: false,
        }
    }

    pub fn reset(&mut self) {
        self.last_tick = std::time::Instant::now();
        self.active = false;
    }

    pub fn tick(&mut self) -> bool {
        let now = std::time::Instant::now();
        if self.last_tick + self.rate > now {
            self.active = false;
        } else {
            self.timer.rearm();
            self.last_tick = now;
            self.active = true;
        }
        self.active
    }

    #[inline(always)]
    pub fn id(&self) -> u8 {
        self.timer.si_value
    }
}
#[test]
fn test_rate_limit() {
    use std::sync::{Arc, Condvar, Mutex};
    /* RateLimit sends a SIGALRM with its timer value in siginfo_t. */
    let pair = Arc::new((Mutex::new(None), Condvar::new()));
    let pair2 = pair.clone();

    /* self-pipe trick:
     * since we can only use signal-safe functions in the signal handler, make a pipe and
     * write one byte to it from the handler. Counting the number of bytes in the pipe can tell
     * us how many times the handler was called */
    let (alarm_pipe_r, alarm_pipe_w) = nix::unistd::pipe().unwrap();
    nix::fcntl::fcntl(
        alarm_pipe_r,
        nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
    )
    .expect("Could not set pipe to NONBLOCK?");

    let alarm_handler = move |info: &nix::libc::siginfo_t| {
        let value = unsafe { info.si_value().sival_ptr as u8 };
        let (lock, cvar) = &*pair2;
        let mut started = lock.lock().unwrap();
        /* set mutex to timer value */
        *started = Some(value);
        /* notify condvar in order to wake up the test thread */
        cvar.notify_all();
        nix::unistd::write(alarm_pipe_w, &[value]).expect("Could not write inside alarm handler?");
    };
    unsafe {
        signal_hook_registry::register_sigaction(signal_hook::SIGALRM, alarm_handler).unwrap();
    }
    /* Accept at most one request per 3 milliseconds */
    let mut rt = RateLimit::new(1, 3);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let (lock, cvar) = &*pair;
    let started = lock.lock().unwrap();
    let result = cvar
        .wait_timeout(started, std::time::Duration::from_millis(100))
        .unwrap();
    /* assert that the handler was called with rt's timer id */
    assert_eq!(*result.0, Some(rt.id()));
    drop(result);
    drop(pair);

    let mut buf = [0; 1];
    nix::unistd::read(alarm_pipe_r, buf.as_mut()).expect("Could not read from self-pipe?");
    /* assert that only one request per 3 milliseconds is accepted */
    for _ in 0..5 {
        assert!(rt.tick());
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(!rt.tick());
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(!rt.tick());
        std::thread::sleep(std::time::Duration::from_millis(1));
        /* How many times was the signal handler called? We've slept for at least 3
         * milliseconds, so it should have been called once */
        let mut ctr = 0;
        while nix::unistd::read(alarm_pipe_r, buf.as_mut())
            .map(|s| s > 0)
            .unwrap_or(false)
        {
            ctr += 1;
        }
        assert_eq!(ctr, 1);
    }
    /* next, test at most 100 requests per second */
    let mut rt = RateLimit::new(100, 1000);
    for _ in 0..5 {
        let mut ctr = 0;
        for _ in 0..500 {
            if rt.tick() {
                ctr += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        /* around 100 requests should succeed. might be 99 if in first loop, since
         * RateLimit::new() has a delay */
        assert!(ctr > 97 && ctr < 103);
        /* alarm should expire in 1 second */
        std::thread::sleep(std::time::Duration::from_millis(1000));
        /* How many times was the signal handler called? */
        ctr = 0;
        while nix::unistd::read(alarm_pipe_r, buf.as_mut())
            .map(|s| s > 0)
            .unwrap_or(false)
        {
            ctr += 1;
        }
        assert_eq!(ctr, 1);
    }
    /* next, test at most 500 requests per second */
    let mut rt = RateLimit::new(500, 1000);
    for _ in 0..5 {
        let mut ctr = 0;
        for _ in 0..500 {
            if rt.tick() {
                ctr += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        /* all requests should succeed.  */
        assert!(ctr < 503 && ctr > 497);
        /* alarm should expire in 1 second */
        std::thread::sleep(std::time::Duration::from_millis(1000));
        /* How many times was the signal handler called? */
        ctr = 0;
        while nix::unistd::read(alarm_pipe_r, buf.as_mut())
            .map(|s| s > 0)
            .unwrap_or(false)
        {
            ctr += 1;
        }
        assert_eq!(ctr, 1);
    }
}

#[derive(Debug)]
pub enum ContactEvent {
    CreateContacts(Vec<melib::Card>),
}

#[derive(Debug)]
pub enum ComposeEvent {
    SetReceipients(Vec<melib::Address>),
}

pub type UIMessage = Box<dyn 'static + std::any::Any + Send + Sync>;
