//! The environment object of each context-group script, handed to its `manivelle:env`
//! module.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use rquickjs::{Ctx, Object, Persistent};

type EnvMap = RefCell<HashMap<u64, Persistent<Object<'static>>>>;

/// Environment objects by graph id. The engine owns it and drops it before the runtime;
/// the loader only holds a weak [`EnvHandle`], because a persistent value must not
/// outlive the runtime that owns the loader.
#[derive(Default)]
pub(crate) struct ScriptEnvs(Rc<EnvMap>);

impl ScriptEnvs {
    pub(crate) fn insert(&self, graph_id: u64, env: Persistent<Object<'static>>) {
        self.0.borrow_mut().insert(graph_id, env);
    }

    pub(crate) fn remove(&self, graph_id: u64) {
        self.0.borrow_mut().remove(&graph_id);
    }

    pub(crate) fn handle(&self) -> EnvHandle {
        EnvHandle(Rc::downgrade(&self.0))
    }
}

/// What the module loader reads environments through.
pub(crate) struct EnvHandle(Weak<EnvMap>);

impl EnvHandle {
    pub(super) fn env<'js>(&self, ctx: &Ctx<'js>, graph_id: u64) -> Option<Object<'js>> {
        let envs = self.0.upgrade()?;
        let env = envs.borrow().get(&graph_id)?.clone();
        env.restore(ctx).ok()
    }
}
