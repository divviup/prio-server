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

### Finding your pod

Suppose you have identified a Kubernetes pod for a `facilitator` process you want to debug. Let's say the pod's name is `facilitator-pod`. You can pass a filter to `ctr containers list` to search for running containers corresponding to the pod.

    gcp-instance> ctr -n k8s.io containers list labels."io.kubernetes.pod.name"==facilitator-pod
    CONTAINER                                                           IMAGE                                                                                       RUNTIME
    32420cd369cf0de3aa70a61df5ea1b2882e0a97ca18e0d5e45afb056f47a1396    docker.io/letsencrypt/prio-facilitator:0.6.21                                               io.containerd.runc.v2
    ad7864f701d16173aac76ad3a0637c2c1107fdea8e1effa48d9d3c24f77a3f96    docker.io/letsencrypt/prio-facilitator:0.6.21                                               io.containerd.runc.v2
    b99a27e135ef1b3475ec8259b3ca6bbd4a0ff7c0e72fb4c85d17847209292279    k8s.gcr.io/pause@sha256:927d98197ec1141a368550822d18fa1c60bdae27b78b0c004f705f548c07814f    io.containerd.runc.v2

That will list all the containers in the named Kubernetes pod. In particular, you want the container ID for one of the `prio-facilitator` entries (your humble author does not know why there are two since there is only one actual `facilitator` process but they both refer to the same sandbox). Now, obtain that conatiner's _sandbox ID_, and use that to obtain the `pid` of the `containerd-shim-runc-v2` process supervising the `facilitator`, and in turn get the `pid`s of its children to finally find the `pid` you want to debug:

    gcp-instance> ctr -n k8s.io containers info ad7864f701d16173aac76ad3a0637c2c1107fdea8e1effa48d9d3c24f77a3f96 | grep io.kubernetes.cri.sandbox-id
                "io.kubernetes.cri.sandbox-id": "b99a27e135ef1b3475ec8259b3ca6bbd4a0ff7c0e72fb4c85d17847209292279"
    gcp-instance> ps aux | grep b99a27e135ef1b3475ec8259b3ca6bbd4a0ff7c0e72fb4c85d17847209292279
    timg      531422  0.0  0.0   6484   832 pts/0    S+   20:42   0:00 grep --colour=auto b99a27e135ef1b3475ec8259b3ca6bbd4a0ff7c0e72fb4c85d17847209292279
    root      670053  0.0  0.0 113404 11068 ?        Sl   Jan11   2:15 /usr/bin/containerd-shim-runc-v2 -namespace k8s.io -id b99a27e135ef1b3475ec8259b3ca6bbd4a0ff7c0e72fb4c85d17847209292279 -address /run/containerd/containerd.sock
    gcp-instance> ps --ppid 670053
    PID TTY          TIME CMD
    670075 ?        00:00:00 pause
    765973 ?        00:00:08 facilitator

[`pause` is a special container inserted by Kubernetes](https://groups.google.com/g/kubernetes-users/c/jVjv0QK4b_o). The one you want is `facilitator`.

### `facilitator` core dumps on GKE

Now that you have the `pid` of the process to debug or take a core from, activate a `toolbox` and install `gdb`:

    gcp-instance> toolbox
    toolbox> apt update && apt-get install gdb
    toolbox> gcore -o <filename for core file> <pid>

You can also use `gdb` to attach to a live process. We don't currently support getting core dumps from crashed processes and uploading them anywhere. Files created in the `toolbox` are visible in the root mount namespace at `/var/lib/toolbox/`.

To get the corefile off of the worker node, you can use `gcloud compute scp` from your workstation:

    workstation> gcloud --project <YOUR PROJECT> compute scp --zone <INSTANCE ZONE> <INSTANCE NAME>:/path/on/instance /local/path

## SSH into EKS worker node

_I don't know how to do this yet -timg_

## Symbolicating a `facilitator` core dump

To make sense of a corefile, you need `gdb` and debug symbols for the images. Your best bet is to run `gdb` from inside the `facilitator` container.

    workstation> docker pull letsencrypt/prio-facilitator:0.x.y
    workstation> docker run -it --entrypoint="/bin/sh" letsencrypt/prio-facilitator:0.x.y

Now you should have an interactive shell inside a `prio-facilitator` container. We build these from Alpine Linux, so you have a basic set of tools as well as a package manager available. Install `gdb` and debug symbols for Alpine's `musl`:

    facilitator> apk update && apk add gdb musl-dbg

Then, from outside the container, copy in the corefile you obtained from the worker. First, get the ID of the container you ran:

    workstation> docker ps

Then:

    workstation> docker cp /path/to/corefile <container ID>:/corefile

Back inside the container, debug the corefile using the symbols in the `facilitator` binary:

    facilitator> gdb /facilitator /corefile
    gdb> thread apply all bt

Note that this only works with sufficiently recent `facilitator` images. Old enough ones strip debug information and thus cannot be symbolicated, though you may have luck symbolicating the `libc` frames. Current `facilitator` builds are not stripped, but we don't include full debug info (that would increase binary size by about 10x; see [this issue](https://github.com/abetterinternet/prio-server/issues/1279) for details). Still, you can at least get symbolicated backtraces, just not line numbers.
