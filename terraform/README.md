# Prio server Terraform module

This Terraform module manages a [GKE cluster](https://cloud.google.com/kubernetes-engine/docs) which hosts a Prio data share processor. We create one cluster and one node pool in each region in which we operate, and then run each PHA's data share processor instance in its own Kubernetes namespace. Each data share processor consists of a [Kubernetes CronJob](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) that runs a workflow manager.

You will need these tools:

- [terraform](https://learn.hashicorp.com/tutorials/terraform/install-cli): To make Terraform create Google Cloud Platform and Kubernetes resources.
- [gcloud](https://cloud.google.com/sdk/docs/install): For interacting with Google Cloud Platform.
- [kubectl](https://kubernetes.io/docs/tasks/tools/install-kubectl/): To manage the Kubernetes clusters, jobs, pods, etc.
- [aws-cli](https://aws.amazon.com/cli/): For interacting with Amazon Web Services through Terraform.
- [helm](https://helm.sh/): To install packaged software on Kubernetes.

We configure the [GCP Terraform provider](https://www.terraform.io/docs/providers/google/index.html) to use the credentials file managed by `gcloud`. To ensure you have well-formed credentials available, do `gcloud auth application-default login` and walk through the authentication flow.

We use the [AWS Terraform provider](https://registry.terraform.io/providers/hashicorp/aws/latest/docs) to use the credentials file managed by aws-cli (`aws`). To ensure you have well-formed credentials available, do `aws iam get-user` and make sure it displays your user information. If it doesn't, use `aws configure` to setup your user. You will need to create an [Access Key](https://console.aws.amazon.com/iam/home#/security_credentials) to configure aws.

We currently use the profile `leuswest2` in terraform (this value is defined in `main.tf`). In order for terraform to recognize your credentials, you will need to configure it with `aws configure --profile leuswest2`.

`helm` is not required for creating or maintaining a cluster, but it's still useful to have configured. Once you have helm installed, add the "stable" repository:

```
helm repo add stable https://charts.helm.sh/stable
```

We use a [Terraform remote backend](https://www.terraform.io/docs/backends/index.html) to manage the state of the deployment, and the state file resides in a [Google Cloud Storage](https://cloud.google.com/storage/docs) bucket. `terraform/Makefile` is set up to manage the remote state, including creating buckets as needed. To use the `Makefile` targets, set the `ENV` environment variable to something that matches one of the `tfvars` files in `terraform/variables`. For instance, `ENV=demo-gcp make plan` will  source Terraform state from the remote state for `demo-gcp`. Try `ENV=<anything> make help` to get a list of targets.

If you have everything set up correctly, you should be able to...

- `ENV=demo-gcp make plan` to get an idea of what Terraform thinks the world looks like
- `gcloud container clusters list` to list GKE clusters
- `kubectl -n test-pha-1 get pods` to list the pods in namespace `test-pha-1`

If you're having problems, check `gcloud config list` and `kubectl config current-context` to ensure you are using the right configurations for the cluster you are working with.

## New clusters

To add a data share processor to support a new locality, add that locality's name to the `localities` variable in the relevant `variables/<environment>.tfvars` file.

To bring up a whole new cluster, drop a `your-new-environment.tfvars` file in `variables`, fill in the required variables and then bootstrap it with:

    ENV=your-new-environment make apply-bootstrap

This will deploy just enough of an environment to permit peers to begin deploying resources. Once your environment is bootstrapped, and once all the other servers you intend to exchange data with have bootstrapped, finish the deploy with

    ENV=your-new-environment make apply

Once bootstrapped, subsequent deployments should use `ENV=your-new-environment make apply`. Multiple environments may be deployed to the same GCP region.

## Paired test environments

We have support for creating two paired test environments which can exchange validation shares, along with a convincing simulation of ingestion servers and a portal server. To do this, you will need to create two `.tfvars` files, and on top of the usual variables, each must contain a variable like:

    test_peer_environment = {
      env_with_ingestor    = "with-ingestor"
      env_without_ingestor = "without-ingestor"
    }

The values must correspond to the names of the environments you are using. Pick one of them to be the environment with ingestors. From there, you should be able to bring up the two environments like so:

    ENV=with-ingestor make apply-bootstrap
    ENV=without-ingestor make apply-bootstrap

After the successful `apply-bootstrap` you may need to wait several minutes for managed TLS certificates to finish provisioning. Once those are in place, move on to the full deployment:

    ENV=with-ingestor make apply
    ENV=without-ingestor make apply

In your test setup, you might want to exercise reading and writing data to AWS S3 buckets, to simulate interacting with a peer data share processor that runs in AWS. Add `use_aws = true` to the `.tfvars` file for the environment you specified as `env_without_ingestor` and that env's ingestion and peer validation buckets will be created in S3.

## kubectl configuration

When instantiating a new GKE cluster, you will want to merge its configuration into your local `~/.kube/config` so that you can use `kubectl` for cluster management. After a successful `apply`, Terraform will emit a `gcloud` invocation that will update your local config file. More generally, `gcloud container clusters get-credentials <YOUR CLUSTER NAME> --region <GCP REGION>"` will do the trick.

## Metrics, graphs and alerting

We deploy Prometheus and Grafana into Prio clusters. A variety of Kubernetes metrics automagically are recorded into Prometheus, and a small number of metrics are recorded by `workflow-manager` and `facilitator`. There are some manual steps before you can see any graphs or get alerts, though.

### Accessing Prometheus and Grafana

None of Grafana, Prometheus or any of Prometheus' components are exposed publicly. You can access them using `kubectl port-forward` to make a port on remote pod's network accessible locally. So, to make Prometheus' `server` component reachable at `localhost:8080`, you would do:

    kubectl port-forward $(kubectl get pod --selector="app=prometheus,component=server,release=prometheus" --output jsonpath='{.items[0].metadata.name}') 8080:9090

For `prometheus-alertmanager`:

    kubectl port-forward $(kubectl get pod --selector="app=prometheus,component=alertmanager,release=prometheus" --output jsonpath='{.items[0].metadata.name}') <LOCAL PORT>:9093

And for Grafana:

    kubectl port-forward $(kubectl get pod --selector="app.kubernetes.io/instance=grafana,app.kubernetes.io/name=grafana" --output jsonpath='{.items[0].metadata.name}') <LOCAL PORT>:3000

To access other components, look up the labels on the pod for the component you want to reach to construct an appropriate `--selector` argument, and check the running container's configuration to see what port(s) it listens on.

Grafana requires authentication. You can login as the `admin` user. The password is in a Kubernetes secret. Run `kubectl -n default get secret grafana -o=yaml`, then base64 decode the value for `admin-password` and use it to authenticate.

### Hooking up VictorOps/Splunk Oncall alerts

Sending alerts from Prometheus Alertmanager to VictorOps requires an API key, which we do not want to store in source control, and so you must update a Kubernetes secret. First, get the appropriate routing key and API key from the [VictorOps portal](https://portal.victorops.com) (configuring VictorOps teams, rotations, and escalation policies is out of scope for this document). Then, make a YAML file like this:

    global:
      resolve_timeout: 5m
      http_config: {}
      victorops_api_url: https://alert.victorops.com/integrations/generic/20131114/alert/
    route:
      receiver: victorops-receiver
      group_by: ['alertname', 'cluster', 'service']
      group_wait: 10s
      group_interval: 5m
      repeat_interval: 3h
    receivers:
    - name: victorops-receiver
      victorops_configs:
      - api_key: <VICTOROPS SECRET API KEY HERE>
        routing_key: <VICTOROPS ROUTING KEY HERE>
        state_message: 'Alert: {{ .CommonLabels.alertname }}. Summary:{{ .CommonAnnotations.summary }}. RawData: {{ .CommonLabels }}'
    templates: []

Next, we want to store this in the `prometheus-alertmanager-config` Kubernetes secret that was created by Terraform with a dummy value. There's no straightforward way to update a secret value using kubectl, so we use [this trick](https://blog.atomist.com/updating-a-kubernetes-secret-or-configmap/):

    kubectl -n default create secret generic prometheus-alertmanager-config --from-file=alertmanager.yml=/path/to/config/file.yml --dry-run=client -o=json | kubectl apply -f -

For more details, see the [VictorOps docs on Prometheus integration](https://help.victorops.com/knowledge-base/victorops-prometheus-integration/).

### Configuring Kubernetes dashboard in Grafana

No dashboards are automatically configured in Grafana. To get [a dashboard with an overview of Kubernetes status](https://grafana.com/grafana/dashboards/315):

* Log into Grafana (see "Accessing Prometheus and Grafana", above)
* Go to Dashboards -> Manage
* Hit "Import"
* Enter "315" into the "Import via grafana.com" box (or the ID of whatever dashboard you wish to configure)
* Select the "prometheus" data source
* Enjoy the graphs'n'gauges

## Formatting

Our continuous integration is set up to do basic validation of Terraform files on pull requests. Besides any other testing, make sure to run `terraform fmt --recursive` or you will get build failures!
