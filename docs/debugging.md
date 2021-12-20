# Debugging

## SSH into GKE worker node

You should be able to SSH into GKE worker nodes using `gcloud`. First, double check your cluster's firewall rules.

    workstation> gcloud --project <YOUR PROJECT> compute firewall-rules list

Terraform should have created a rule allowing ingress on port 22 on the network your cluster uses (try `gcloud container clusters describe <YOUR CLUSTER>` to see what network your cluster uses). If not, you can make the rule yourself:

    workstation> gcloud --project <YOUR PROJECT> compute firewall-rules create <YOUR CLUSTER>-ssh --allow tcp:22 --network <YOUR NETWORK>

Now, you should be able to get a shell on a worker node. To discover your worker nodes and their zones:

    workstation> gcloud --project <YOUR PROJECT> compute instances list
    workstation> gcloud --project <YOUR PROJECT> compute ssh --zone <INSTANCE ZONE> <INSTANCE>

And you should get a shell.

Google's Container Optimized OS (COS) uses [`containerd`](https://github.com/containerd/containerd) to supervise Kubernetes containers. To interact with `containerd`, you can use the `ctr` utility, which is installed by default. Note that `ctr` has a concept of "namespaces" and the Kubernetes containers you want to examine are likely to be in the `k8s.io` namespace. Something like `ctr --namespace=k8s.io containers list` should work.

Other utilities like `gdb` aren't present in COS. Fortunately, it provides a utility called [`toolbox`](https://cloud.google.com/container-optimized-os/docs/how-to/toolbox) which gets you a Debian based environment where you can install whatever you want. The `toolbox` session's filesystem may be accessed from the root mount namespace at `/var/lib/toolbox/<session name>`.

### `facilitator` core dumps on GKE

In order to get a core dump of a running `facilitator` from a GKE node, you will first need to SSH in, as described above. Then, use `ctr containers describe` to figure out the `pid` of the process you want to debug. Then, activate a `toolbox` and install `gdb`:

    toolbox> apt update && apt-get install gdb
    toolbox> gcore -o <filename for core file> <pid>

You can also use `gdb` to attach to a live process. We don't currently support getting core dumps from crashed processes and uploading them anywhere. Files created in the `toolbox` are visible in the root mount namespace at `var`

To get the corefile off of the worker node, you can use `gcloud compute scp` from your workstation:

    workstation> gcloud --project <YOUR PROJECT> compute scp --zone <INSTANCE ZONE> <INSTANCE NAME>:/path/on/instance /local/path

## SSH into EKS worker node

_I don't know how to do this yet -timg_

## Symbolicating a `facilitator` core dump

To make sense of a corefile, you need `gdb` and debug symbols for the images. Your best bet is to run `gdb` from inside the `facilitator` container.

    workstation> docker pull letsencrypt/prio-facilitator:0.x.y
    workstation> docker run -it --entrypoint="/bin/sh" letsencrypt/prio-facilitator:0.6.20

Now you should have an interactive shell inside a `prio-facilitator` container. We build these from Alpine Linux, so you have a basic set of tools as well as a package manager available. Install `gdb` and debug symbols for Alpine's `musl`:

    facilitator> apk update && apk add gdb musl-dbg

Then, from outside the container, copy in the corefile you obtained from the worker. First, get the ID of the container you ran:

    workstation> docker ps

Then:

    workstation> docker cp /path/to/corefile <container ID>:/corefile

Back inside the container, debug the corefile using the symbols in the `facilitator` binary:

    facilitator> gdb /facilitator /corefile

Note that this only works with sufficiently recent `facilitator` images. Old enough ones strip debug information and thus cannot be symbolicated, though you may have luck symbolicating the `libc` frames.
