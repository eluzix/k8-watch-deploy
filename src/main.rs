use std::{borrow::Borrow, str::FromStr};

// use futures::{StreamExt, TryStreamExt};
use clap::{Parser, Subcommand};
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, PartialObjectMeta, ResourceExt, WatchEvent, WatchParams},
    runtime::{watcher, WatchStreamExt},
    Client,
};
use mac_notification_sys::*;
use std::pin::pin;
use tokio::sync::Notify;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, group = "resource")]
    pod: Option<String>,
    #[arg(short, long, group = "resource")]
    release: Option<String>,
    #[arg(short, long, default_value_t = String::from_str("application").unwrap())]
    namespace: String,
}

#[derive(Debug)]
struct FlatPodStatus {
    phase: String,
    reason: Option<String>,
}

impl std::fmt::Display for FlatPodStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = self.reason.borrow() {
            write!(f, "{:?} ({:?})", self.phase, reason)
        } else {
            write!(f, "{:?}", self.phase)
        }
    }
}

fn notify(title: &str, msg: &str) {
    send_notification(title, None, msg, Some(Notification::new().sound("Funk"))).unwrap();
}

async fn get_pod_info(client: &Client, ns: &str, pod_name: &str) -> Option<PartialObjectMeta<Pod>> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    // let pod = pods.get(pod_name).await.ok()?;
    let pod: PartialObjectMeta<Pod> = pods.get_metadata(pod_name).await.ok()?;

    // println!(">>>>>> pod: {:?}", pod);
    // for l in pod.labels().iter() {
    //     println!("----> {}: {}", l.0, l.1);
    // }

    Some(pod)
}

fn get_instance_label(pod: &PartialObjectMeta<Pod>) -> Option<String> {
    for label in pod.labels() {
        if label.0 == "app.kubernetes.io/instance" {
            return Some(label.1.into());
        }
    }

    None
}

fn extract_pod_status(pod: &Pod) -> FlatPodStatus {
    if let Some(status) = pod.status.borrow() {
        let phase = status.phase.clone().unwrap_or("Unknown".to_string());
        let reason = status.reason.clone();
        FlatPodStatus { phase, reason }
    } else {
        FlatPodStatus {
            phase: "Unknown".to_string(),
            reason: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let bundle = get_bundle_identifier_or_default("lens");
    set_application(&bundle).unwrap();

    let ns = &cli.namespace;
    let mut release_name: Option<String> = None;

    if let Some(cli_release) = cli.release {
        release_name = Some(cli_release.clone());
    } else if let Some(cli_pod) = cli.pod {
        let client = Client::try_default().await?;
        if let Some(pod_metadata) = get_pod_info(&client, ns, &cli_pod).await {
            release_name = get_instance_label(&pod_metadata);
        }
    }

    if release_name.is_none() {
        eprintln!("Missing pod/release name");
        return Ok(());
    }

    // Infer the runtime environment and try to create a Kubernetes Client
    let client = Client::try_default().await?;

    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    // for p in pods.list(&ListParams::default()).await? {
    //     println!("found pod {}", p.name_any());
    // }

    let instance_label = format!("app.kubernetes.io/instance={}", release_name.unwrap());

    // notify(
    //     "k9s deployment watcher",
    //     &format!("watching for {}", &instance_label),
    // );

    // let wp = WatchParams::default().labels(&instance_label).timeout(5);
    // let wp = WatchParams::default().labels(&instance_label);
    // let mut stream = pods.watch(&wp, "0").await?.boxed();
    // while let Some(status) = stream.try_next().await? {
    //     match status {
    //         WatchEvent::Added(s) => println!("Added: {}", s.name_any()),
    //         WatchEvent::Modified(s) => println!("Modified: {}", s.name_any()),
    //         WatchEvent::Deleted(s) => println!("Deleted: {}", s.name_any()),
    //         WatchEvent::Error(s) => println!("ERROR: {}", s),
    //         _ => {}
    //     }
    // }

    let wc = watcher::Config::default()
        .labels(&instance_label)
        .timeout(9);
    let stream = watcher(pods, wc).applied_objects();
    let stream = pin!(stream);
    let _ = stream
        .try_for_each(|p| async move {
            let pod_status = extract_pod_status(&p);
            println!("POD: {}, status: {}", p.name_any(), pod_status);
            if pod_status.phase != "Running" {
                notify(
                    &format!("Pod {}", p.name_any()),
                    &format!("Is in non 'Running' phase: {}", pod_status),
                );
            }
            Ok(())
        })
        .await;

    println!("Done.");

    Ok(())
}
