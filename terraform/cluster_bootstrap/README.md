# `cluster_bootstrap`

This Terraform module is responsible for creating managed Kubernetes cluster in either [Google Kubernetes Engine](https://cloud.google.com/kubernetes-engine) or [Elastic Kubernetes Service](https://aws.amazon.com/eks/), depending on the environment. We do this in a separate Terraform project because Hashicorp [strongly recommends not configuring a provider with the output of other resources](https://github.com/abetterinternet/prio-server/issues/1046).

For usage instructions, see the README one level up, or the [`prio-server` onboarding guide](https://github.com/abetterinternet/docs/blob/main/prio-server/onboarding.md).
