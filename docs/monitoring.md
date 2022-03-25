# Metrics, graphs and alerting

We deploy Prometheus and Grafana into Prio clusters. We gather metrics from a few sources:

 - host level metrics via [`prometheus-node-exporter`](https://prometheus.io/docs/guides/node-exporter/)
 - Kubernetes metrics via [`prometheus-kube-state-metrics`](https://github.com/kubernetes/kube-state-metrics)
 - [GCP Cloud Monitoring](https://cloud.google.com/monitoring) via [`stackdriver-exporter`](https://github.com/prometheus-community/stackdriver_exporter)
 - `workflow-manager` pushes metrics via [`prometheus-pushgateway`](https://prometheus.io/docs/instrumenting/pushing/)
 - `facilitator` serves metrics from a `/metrics` endpoint on a configurable port.

## Dealing with alerts

The `prod-us` and `staging` environments are configured to deliver alerts to VictorOps. By default, other environments will track alerts in `prometheus-alertmanager`, but no alerts will be forwarded to VictorOps. Regardless of environment, alerts are tracked by `prometheus-alertmanager`.

To silence alerts, use the `prometheus-alertmanager` web interface. See [below](#access-prometheus-and-grafana-ui) for guidance on accessing the web interfaces, and [Prometheus documentation](https://prometheus.io/docs/alerting/latest/alertmanager/) for more on Alertmanager silences.

### Dead letter queue alerts

We alert if there are undelivered messages in any dead letter queue for five minutes, meaning that some message has repeatedly failed to be handled by a worker. You can examine the undeliverable messages by examining the relevant dead letter subscription (see the `subscription_id` label in the firing alert) [in the GCP PubSub console](https://console.cloud.google.com/cloudpubsub/subscription/list), and you can dismiss the alert by acking the undeliverable messages in the subscription.

## Storage

We configure a regional Google Compute Engine disk for `prometheus-server`'s storage. Its size is configurable in the an environment's `tfvars` file. the GCE disk is made available to `prometheus-server` as a Kubernetes persistent volume claim.

## Configuration

Most configuration is handled via Terraform, in the `monitoring` module. Individual environment configuration will typically set the `victorops_routing_key` variable to an appropriate value (e.g. a preconfigured routing key in Splunk for production/staging environments, most likely a bogus value for development environments) in the environment's `tfvars` file.

Monitoring can be disabled by setting the `pushgateway` variable to the empty string.

### Alerting rules

Alerting rules are defined in `modules/monitoring/prometheus_alerting_rules.yml`. When writing alerting rules, note carefully the difference between Terraform template variables (e.g., `${environment}`), which are substituted by Terraform during `apply`, and Prometheus template variables (e.g., `{{ $labels.kubernetes_name }}`), which are rendered by Prometheus when an alert fires.

### Authoring alerting rules

Composing a PromQL expression for an alerting rule can be a complicated balance, constrained by what metrics are available from exporters, and the ever-changing cardinality of metrics' label sets. Alerting rules must not fire false positives in the face of normal application behavior, and it's desirable that alert rules not show up as "pending" spuriously, even if they don't go into a false alarm. On the other hand, alerting rules may fail to fire on the behavior they were meant to detect, and this can go unnoticed without positive testing.

#### Case study: facilitator failure rate rules

Previous versions of the failure rate rules were similar to `rate(facilitator_intake_tasks_finished{status="success"}[1h]) / rate(facilitator_intake_tasks_finished[1h]) < 0.95`. This rule operates on a counter metric, and it was intended to alert when the relative rates of success and error events crossed a threshold. However, the left hand side of the comparison unconditionally evaluated to 1.0, with a subset of available labels. The un-decorated division operator will operate on matching label sets from both its arguments, and discard any label sets that don't match. Here, it took all of the success-labeled rates from the numerator, matched them with success-labeled rates from the denominator, and discarded all error-labeled rates, which only appeared in the denominator.

The revised rule fixed this issue by aggregating both the numerator and denominator to get rid of the status labels, as follows: `(sum without (status) (rate(facilitator_intake_tasks_finished{status="success"}[1h]))) / (sum without (status) (rate(facilitator_intake_tasks_finished[1h))) < 0.95`. As an alternate solution, aggregation of the denominator could be combined with a grouping divison operator: `rate(facilitator_intake_tasks_finished{status="success"}[1h]) / ignoring (status) (sum without (status) (rate(facilitator_intake_tasks_finished[1h])))`.

#### Case study: kubernetes_pod_waiting

The first version of this rule had severe false positive behavior. The original rule was `min_over_time(kube_pod_container_status_waiting_reason[15m]) > 0`. The metric `kube_pod_container_status_waiting_reason` is a gauge, and it appears the metric takes a value of 1 for label sets corresponding to any waiting container, while emitting no label sets for any non-waiting container. This is thus an "info metric". The updated rule uses `kube_pod_container_status_waiting` instead, another gauge, which exports a 0 or 1 for each container.

Additionally, just taking `min_over_time()` on the metric may trigger on job containers. If the exporter sees a job container during a brief window when it is in `ContainerCreating` during one scrape, the job completes, and then the exporter sees no label sets for the container during the next scrape, then `min_over_time()` will return 1 for this container's label set, even though the job has completed normally. Thus, the rule now multiplies by two instant vectors, one at each end of the fifteen minute window, so that we will only see label sets for which the container has existed for at least fifteen minutes, and has been waiting for at least that long as well. Put together, the updated expression is `min_over_time(kube_pod_container_status_waiting[15m]) * kube_pod_container_status_waiting * (kube_pod_container_status_waiting offset 15m) > 0`.

#### Case study: kubernetes_pod_restarting

The first version of this rule produced some false positives due to a rarer combination of events. The rule depends on taking `increase()` on a range vector from `kube_pod_container_status_restarts_total`, a counter metric. Counterintuitively, `increase()` works by first calculating `rate()` across a range vector, and then multiplying by the range vector's time interval. Thus, if a label set is only present for a subset of the time interval, the value returned will be a linear extrapolation, and not a simple difference in values. This behavior caused false positives in cases where a container restarted once shortly before it was deleted. This could happen for benign reasons, as all our application deployments restart themselves hourly, and autoscaling can delete pods. The metric `kube_pod_container_status_restarts_total` would look like a single step up, followed by a cutoff where we have no data. In the fifteen minutes following this event, `increase(kube_pod_container_status_restarts_total)` would look like a sawtooth wave overlaid on something like 1/x, as the computed `rate()` appears to get steeper while the window of valid data shrinks. If the time between the restart and pod deletion was short enough, then this would exceed the alerting rule's threshold, and we would get a false positive.

The updated rule incorporated two changes to fix this 1/x-like asymptote, one for each end of the interval. First, we multiply by the info metric `kube_pod_info` as an instant vector. This will knock out any label sets for pods that no longer exist at the current time, and get rid of the potential 1/x behavior fifteen minutes after a pod is deleted. Second, the updated rule includes `for: 5m` in the rule definition, to avoid issues from `increase()` blowing up if a label set is only present on the right side of the interval. If a container starts up, and restarts only once for some benign reason in its first few minutes, then the rule will only be pending and not firing. If the container only restarts once, then `increase()` will decay back down as more data comes in across a wider time span. In the event that a container is truly stuck in a reboot loop, then `increase()` will fluctuate around an asymptote at ~2.7, and the rule will activate.

### Alertmanager

`prometheus-alertmanager` is configured to send alerts to VictorOps. If an environment's alerts should generate VictorOps pages, then the `victorops_routing_key` variable in the environment's `tfvars` should be set to an existing routing key in VictorOps. If no valid routing key is provided, then alerts will still be tracked in `prometheus-alertmanager` and visible in its GUI, but nothing will page.

`alertmanager`'s configuration is stored in the Kubernetes secret `prometheus-alertmanager-config` in the `monitoring` namespace. That config document contains the VictorOps API key, which we do not check into source control (see [below](#hooking-up-victoropssplunk-oncall-alerts) for details on providing the API key). Because Terraform is configured to ignore changes to the contents of the secret, if you make changes to `alertmanager` configuration via Terraform, you will either need to apply those changes to the Kubernetes secret manually, or you will need to delete the secret `monitoring/prometheus-alertmanager-config` so that Terraform can re-create the secret from scratch.

### Stackdriver exporter

`stackdriver-exporter` is configured with an allow-list of GCP Cloud Monitoring/Stackdriver metric name prefixes to be exported to Prometheus. You can export additional Stackdriver metrics by [looking up the prefix(es)](https://cloud.google.com/monitoring/api/metrics_gcp) and appending to the `stackdriver.metrics.typePrefixes` list in `resource.helm_release.stackdriver_exporter` in `monitoring.tf`.

### Ad-hoc configuration changes

For development or incident response purposes, you might want to make rapid changes to `prometheus` configuration without going through Terraform.

If you want to modify `prometheus-server` config or alert definitions, you can modify the relevant key in the `prometheus-server` ConfigMap in the `monitoring` namespace. The various Prometheus components will detect your changes and automatically reload configuration.

If you want to modify `stackdriver-exporter` configuration, you can edit the `stackdriver-exporter-prometheus-stackdriver-exporter` deployment in the `monitoring` namespace as its configuration is provided in environment variables in the container spec.

## Accessing Prometheus and Grafana UI

None of Grafana, Prometheus or any of Prometheus' components are exposed publicly. You can access them using `kubectl port-forward` to make a port on remote pod's network accessible locally. So, to make Prometheus' `server` component reachable at `localhost:8080`, you would do:

    kubectl -n monitoring port-forward $(kubectl -n monitoring get pod --selector="app=prometheus,component=server,release=prometheus" --output jsonpath='{.items[0].metadata.name}') 8080:9090

For `prometheus-alertmanager`:

    kubectl -n monitoring port-forward $(kubectl -n monitoring get pod --selector="app=prometheus,component=alertmanager,release=prometheus" --output jsonpath='{.items[0].metadata.name}') 8081:9093

And for Grafana:

    kubectl -n monitoring port-forward $(kubectl -n monitoring get pod --selector="app.kubernetes.io/instance=grafana,app.kubernetes.io/name=grafana" --output jsonpath='{.items[0].metadata.name}') 8082:3000

To access other components, look up the labels on the pod for the component you want to reach to construct an appropriate `--selector` argument, and check the running container's configuration to see what port(s) it listens on. Naturally, this requires you to have valid Kubernetes cluster credentials in place for use with `kubectl`.

Grafana requires authentication. You can login as the `admin` user, whose password is in a Kubernetes secret. Run `kubectl -n monitoring get secret grafana -o=yaml`, then base64 decode the value for `admin-password` and use it to authenticate.

## Hooking up VictorOps/Splunk Oncall alerts

Sending alerts from Prometheus Alertmanager to VictorOps requires an API key, which we do not want to store in source control, and so you must update a Kubernetes secret. First, get the API key from the [VictorOps portal](https://portal.victorops.com) (configuring VictorOps teams, rotations, and escalation policies is out of scope for this document). Then, get the existing `prometheus-alertmanager` config out of the Kubernetes secret:

    kubectl -n monitoring get secrets  prometheus-alertmanager-config -o jsonpath="{.data.alertmanager\.yml}" | base64 -d > /path/to/config/file.yml

Update the `api_key` value in the `victorops_config` and `apply` the secret back into place. There's no straightforward way to update a secret value using kubectl, so we use [this trick](https://blog.atomist.com/updating-a-kubernetes-secret-or-configmap/):

    kubectl -n monitoring create secret generic prometheus-alertmanager-config --from-file=alertmanager.yml=/path/to/config/file.yml --dry-run=client -o=json | kubectl apply -f -

For more details, see the [VictorOps docs on Prometheus integration](https://help.victorops.com/knowledge-base/victorops-prometheus-integration/).

## Configuring Kubernetes dashboard in Grafana

No dashboards are automatically configured in Grafana. To get [a dashboard with an overview of Kubernetes status](https://grafana.com/grafana/dashboards/315):

* Log into Grafana (see "Accessing Prometheus and Grafana", above)
* Go to Dashboards -> Manage
* Hit "Import"
* Enter "315" into the "Import via grafana.com" box (or the ID of whatever dashboard you wish to configure)
* Select the "prometheus" data source
* Enjoy the graphs'n'gauges
