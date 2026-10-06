//! What can be forwarded from a pod, a Service, a Deployment, or a StatefulSet, and how the
//! Port-forward menu item, the F key, and the Forward buttons of the drawers offer it (spec 0035).
//! The decisions are pure and tested; the menu builds its items from them.
//!
//! Every function names the cluster of the row: the gate comes from the guard of that cluster, and
//! a click starts the forward there, never in the primary.

use std::rc::Rc;

use cluster::{ContainerKind, ContainerPort, PodSummary, ServicePortSummary, TemplateContainer};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{AnyElement, App, Context, SharedString, WeakEntity, Window};

use crate::app_shell::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::drawer::{ClickHandler, port_row};
use crate::kind_row::{KindObject, KindRow};
use crate::pod_drawer::kind_tag_text;
use crate::port_forwards::{ForwardId, PortForwards, TargetKind, TargetSpec};
use crate::resource_actions::{
    ActionAvailability, ResourceAction, RowAction, action_availability, action_label,
    disabled_menu_item, row_keyed,
};
use crate::write_guard::ClusterGuard;

/// The call an entry of the Port-forward menu makes when it is clicked.
type StartPort = Box<dyn Fn(&mut Window, &mut App)>;

const UDP_REASON: &str = "UDP ports cannot be forwarded";
const NO_SELECTOR_REASON: &str = "Forward a pod: this Service selects no pods";

/// One declared port of a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortChoice {
    /// The text of the menu item.
    pub(crate) label: String,
    pub(crate) remote_port: u16,
    pub(crate) is_tcp: bool,
}

/// A forward target with the ports it declares, read from the live row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ForwardSubject {
    pub(crate) namespace: String,
    pub(crate) target: TargetSpec,
    pub(crate) ports: Vec<PortChoice>,
    /// Why nothing can be forwarded to this target at all.
    pub(crate) blocked: Option<&'static str>,
}

fn is_tcp(protocol: &str) -> bool {
    protocol.eq_ignore_ascii_case("TCP")
}

fn container_port_text(port: &ContainerPort) -> String {
    let name = port
        .name
        .as_ref()
        .map(|name| format!("{name} "))
        .unwrap_or_default();
    format!("{name}{}/{}", port.port, port.protocol)
}

/// One item per declared container port, with the container and its MAIN/SIDECAR tag. Init
/// containers run before the pod serves, so they are not offered.
pub(crate) fn pod_subject(pod: &PodSummary) -> ForwardSubject {
    let ports = pod
        .containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
        .flat_map(|container| {
            container.ports.iter().map(move |port| PortChoice {
                label: format!(
                    "{} · {} · {}",
                    container.name,
                    container_port_text(port),
                    kind_tag_text(container.kind)
                ),
                remote_port: port.port,
                is_tcp: is_tcp(&port.protocol),
            })
        })
        .collect();
    ForwardSubject {
        namespace: pod.namespace.clone(),
        target: TargetSpec::pod(&pod.name),
        ports,
        blocked: None,
    }
}

fn template_ports(containers: &[TemplateContainer]) -> Vec<PortChoice> {
    containers
        .iter()
        .flat_map(|container| {
            container.ports.iter().map(move |port| PortChoice {
                label: format!("{} · {}", container.name, container_port_text(port)),
                remote_port: port.port,
                is_tcp: is_tcp(&port.protocol),
            })
        })
        .collect()
}

fn service_ports(ports: &[ServicePortSummary]) -> Vec<PortChoice> {
    ports
        .iter()
        .map(|port| {
            let name = port
                .name
                .as_ref()
                .map(|name| format!("{name} "))
                .unwrap_or_default();
            PortChoice {
                label: format!("{name}{}/{}", port.port, port.protocol),
                remote_port: port.port,
                is_tcp: is_tcp(&port.protocol),
            }
        })
        .collect()
}

/// The subject of a Services, Deployments, or StatefulSets row; `None` for any other kind.
pub(crate) fn row_subject(row: &KindRow) -> Option<ForwardSubject> {
    let namespace = row.namespace.clone()?;
    let name = row.name.clone();
    let (kind, ports, blocked) = match &row.object {
        KindObject::Service(service) => {
            let selects_nothing =
                service.selector.is_empty() || service.service_type == "ExternalName";
            (
                TargetKind::Service,
                service_ports(&service.ports),
                selects_nothing.then_some(NO_SELECTOR_REASON),
            )
        }
        KindObject::Deployment(deployment) => (
            TargetKind::Deployment,
            template_ports(&deployment.containers),
            None,
        ),
        KindObject::StatefulSet(set) => (
            TargetKind::StatefulSet,
            template_ports(&set.containers),
            None,
        ),
        _ => return None,
    };
    Some(ForwardSubject {
        namespace,
        target: TargetSpec { kind, name },
        ports,
        blocked,
    })
}

/// What the Port-forward item of a menu, and the F key, do for one subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MenuState {
    Disabled(SharedString),
    /// One TCP port and nothing else: start at once.
    Direct(PortChoice),
    /// Several ports: a submenu, a UDP port disabled.
    Pick(Vec<PortChoice>),
    /// No port is declared: New forward opens with the target filled in.
    NewForward,
}

/// Decided from the gate of the subject's own cluster and the ports it declares. Pure.
pub(crate) fn menu_state(subject: &ForwardSubject, gate: ActionAvailability) -> MenuState {
    if let ActionAvailability::Disabled { reason } = gate {
        return MenuState::Disabled(reason);
    }
    if let Some(reason) = subject.blocked {
        return MenuState::Disabled(reason.into());
    }
    match subject.ports.as_slice() {
        [] => MenuState::NewForward,
        [only] if only.is_tcp => MenuState::Direct(only.clone()),
        ports if ports.iter().any(|port| port.is_tcp) => MenuState::Pick(ports.to_vec()),
        _ => MenuState::Disabled(UDP_REASON.into()),
    }
}

/// The port the New forward dialog opens with: the first TCP port the target declares.
pub(crate) fn first_tcp_port(subject: &ForwardSubject) -> Option<u16> {
    subject
        .ports
        .iter()
        .find(|port| port.is_tcp)
        .map(|port| port.remote_port)
}

/// The Port-forward item of a menu, with everything it needs owned: a submenu is built from the
/// app, so a caller makes this before it borrows the session (like `ShellMenu`).
pub(crate) struct ForwardMenu {
    state: MenuState,
    cluster: ClusterRef,
    subject: ForwardSubject,
}

impl ForwardMenu {
    pub(crate) fn of(
        subject: ForwardSubject,
        cluster: &ClusterRef,
        guard: &ClusterGuard<'_>,
    ) -> Self {
        Self {
            state: menu_state(
                &subject,
                action_availability(ResourceAction::PortForward, guard),
            ),
            cluster: cluster.clone(),
            subject,
        }
    }

    /// Each entry starts the forward in the row's cluster, through the guarded flow.
    pub(crate) fn item(
        self,
        shell: &WeakEntity<AppShell>,
        window: &mut Window,
        cx: &mut App,
    ) -> PopupMenuItem {
        let label = action_label(ResourceAction::PortForward);
        let Self {
            state,
            cluster,
            subject,
        } = self;
        let start = {
            let (shell, cluster, subject) = (shell.clone(), cluster.clone(), subject.clone());
            move |port: u16| -> StartPort {
                let (shell, cluster, subject) = (shell.clone(), cluster.clone(), subject.clone());
                Box::new(move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.start_forward_port(
                            &cluster,
                            &subject.namespace,
                            &subject.target,
                            port,
                            window,
                            cx,
                        );
                    });
                })
            }
        };
        match state {
            MenuState::Disabled(reason) => {
                row_keyed(disabled_menu_item(label, reason), RowAction::PortForward)
            }
            MenuState::Direct(choice) => {
                let start = start(choice.remote_port);
                row_keyed(
                    PopupMenuItem::new(label).on_click(move |_, window, cx| start(window, cx)),
                    RowAction::PortForward,
                )
            }
            MenuState::NewForward => {
                let shell = shell.clone();
                row_keyed(
                    PopupMenuItem::new(format!("{label}…")).on_click(move |_, window, cx| {
                        let (cluster, subject) = (cluster.clone(), subject.clone());
                        let _ = shell.update(cx, |shell, cx| {
                            shell.open_new_forward_for(&cluster, &subject, window, cx);
                        });
                    }),
                    RowAction::PortForward,
                )
            }
            MenuState::Pick(choices) => {
                let submenu = PopupMenu::build(window, cx, move |submenu, _, _| {
                    choices.iter().fold(submenu, |submenu, choice| {
                        if !choice.is_tcp {
                            return submenu
                                .item(disabled_menu_item(choice.label.clone(), UDP_REASON.into()));
                        }
                        let start = start(choice.remote_port);
                        submenu.item(
                            PopupMenuItem::new(choice.label.clone())
                                .on_click(move |_, window, cx| start(window, cx)),
                        )
                    })
                });
                row_keyed(
                    PopupMenuItem::submenu(label, submenu),
                    RowAction::PortForward,
                )
            }
        }
    }
}

/// `localhost:19090`: what a running forward shows and copies.
pub(crate) fn forward_address_text(local_port: u16) -> String {
    format!("localhost:{local_port}")
}

/// The tooltip of the address of a running forward.
pub(crate) fn copy_address_tooltip(local_port: u16) -> String {
    format!("Copy {}", forward_address_text(local_port))
}

/// How one port of a drawer offers Forward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PortButton {
    /// Click starts a forward with an automatic local port.
    Offer,
    /// A forward of this port runs: the address `● localhost:19090` copies, a Stop button stops.
    Live {
        id: ForwardId,
        local_port: u16,
    },
    Disabled(SharedString),
}

/// What the Forward buttons of one drawer read: the running forwards, the cluster of the drawer
/// subject, and the gate of that cluster. Never the primary's.
pub(crate) struct PortButtons<'a> {
    pub(crate) forwards: &'a PortForwards,
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) gate: ActionAvailability,
}

impl PortButtons<'_> {
    pub(crate) fn button(&self, subject: &ForwardSubject, choice: &PortChoice) -> PortButton {
        if let Some(forward) = self.forwards.running_for(
            self.cluster,
            &subject.namespace,
            &subject.target,
            choice.remote_port,
        ) {
            let local_port = forward
                .local
                .map_or_else(|| forward.spec.requested_local_port(), |a| a.port());
            return PortButton::Live {
                id: forward.id,
                local_port,
            };
        }
        if let ActionAvailability::Disabled { reason } = &self.gate {
            return PortButton::Disabled(reason.clone());
        }
        if let Some(reason) = subject.blocked {
            return PortButton::Disabled(reason.into());
        }
        if !choice.is_tcp {
            return PortButton::Disabled(UDP_REASON.into());
        }
        PortButton::Offer
    }

    /// One port of a drawer with its Forward button, acting on the drawer subject's own cluster.
    pub(crate) fn row(
        &self,
        text: &SharedString,
        id: usize,
        subject: &ForwardSubject,
        choice: &PortChoice,
        cx: &Context<AppShell>,
    ) -> AnyElement {
        let button = self.button(subject, choice);
        let on_click: Option<ClickHandler> = match &button {
            PortButton::Offer => {
                let (cluster, namespace, target) = (
                    self.cluster.clone(),
                    subject.namespace.clone(),
                    subject.target.clone(),
                );
                let port = choice.remote_port;
                Some(Rc::new(cx.listener(move |shell, _, window, cx| {
                    shell.start_forward_port(&cluster, &namespace, &target, port, window, cx);
                })))
            }
            PortButton::Live { id: forward, .. } => {
                let forward = *forward;
                Some(Rc::new(cx.listener(move |shell, _, _, cx| {
                    shell.stop_forward(forward, cx);
                })))
            }
            PortButton::Disabled(_) => None,
        };
        port_row(text, id, &button, on_click, cx)
    }
}

#[cfg(test)]
#[path = "port_forward_menu_tests.rs"]
mod port_forward_menu_tests;
