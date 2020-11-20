/*
Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package controllers

import (
	"context"
	"fmt"
	"k8s.io/apimachinery/pkg/api/equality"
	"strings"

	"github.com/go-logr/logr"
	v1 "k8s.io/api/batch/v1"
	"k8s.io/api/batch/v1beta1"
	v12 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"

	priov1 "github.com/abetterinternet/prio-server/deploy-operator/api/v1"
)

// LocalityReconciler reconciles a Locality object
type LocalityReconciler struct {
	client.Client
	Log    logr.Logger
	Scheme *runtime.Scheme

	ctx context.Context
}

const (
	jobNameField       = ".metadata.name"
	jobName            = "key-rotator"
	keyRotatorImage    = "letsencrypt/prio-manifest-updater:latest"
	serviceAccountName = "manifest-updater"
)

// +kubebuilder:rbac:groups=prio.isrg-prio.org,resources=localities,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=prio.isrg-prio.org,resources=localities/status,verbs=get;update;patch
// +kubebuilder:rbac:groups=batch,resources=cronjobs,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=batch,resources=cronjobs/status,verbs=get
// +kubebuilder:rbac:groups=batch,resources=jobs,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=batch,resources=jobs/status,verbs=get

func (r *LocalityReconciler) Reconcile(req ctrl.Request) (ctrl.Result, error) {
	r.ctx = context.Background()
	r.Log = r.Log.WithValues("locality", req.NamespacedName)

	var locality priov1.Locality
	if err := r.Get(r.ctx, req.NamespacedName, &locality); err != nil {
		r.Log.Error(err, "unable to fetch Locality resource")

		return ctrl.Result{}, client.IgnoreNotFound(err)
	}

	ret, err := r.validate(&locality)
	// Assume the error has been logged and the correct return has been selected
	if err != nil {
		return ret, err
	}

	var cronJobs v1beta1.CronJobList

	if err := r.List(r.ctx, &cronJobs, client.InNamespace(req.Namespace), client.MatchingFields{
		jobNameField: jobName,
	}); err != nil {
		r.Log.Error(err, "unable to fetch list of jobs for controller")

		return ctrl.Result{}, err
	}
	r.Log.Info("List of cronjobs returned", "cronjobs", cronJobs, "cronjobs size", len(cronJobs.Items))
	switch len(cronJobs.Items) {
	case 0:
		ret, err = r.scheduleNewJob(&locality)
		if err != nil {
			return ret, err
		}
	case 1:
		ret, err = r.validateCurrentJob(&cronJobs.Items[0], &locality)
		if err != nil {
			return ret, err
		}

	default:
		panic("THIS SHOULD NEVER HAPPEN!")
	}

	return ctrl.Result{}, nil
}

func (r *LocalityReconciler) scheduleNewJob(locality *priov1.Locality) (ctrl.Result, error) {
	cronJob := r.createCronJobTemplate(locality)

	if err := r.Client.Create(r.ctx, &cronJob); err != nil {
		r.Log.Error(err, "unable to create a new CronJob")
		return ctrl.Result{}, err
	}

	job := r.createJobFromCronJob(&cronJob)
	if err := r.Client.Create(r.ctx, &job); err != nil {
		r.Log.Error(err, "unable to create an immediate job from the cron job")
		return ctrl.Result{}, err
	}

	return ctrl.Result{}, nil
}

func (r *LocalityReconciler) createJobFromCronJob(cronJob *v1beta1.CronJob) v1.Job {
	return v1.Job{
		ObjectMeta: metav1.ObjectMeta{
			Name:      fmt.Sprintf("init-%s", jobName),
			Namespace: cronJob.Namespace,
			Annotations: map[string]string{
				"cronjob.kubernetes.io/instantiate": "manual",
			},
		},
		Spec: *cronJob.Spec.JobTemplate.Spec.DeepCopy(),
	}
}

func (r *LocalityReconciler) createCronJobTemplate(locality *priov1.Locality) v1beta1.CronJob {
	var jobTTL int32
	jobTTL = 5

	trueValue := true

	env := []v12.EnvVar{
		{Name: "KR_ENVIRONMENT_NAME", Value: locality.Spec.EnvironmentName},
		{Name: "KR_MANIFEST_BUCKET_LOCATION", Value: locality.Spec.ManifestBucketLocation},
		{Name: "KR_INGESTORS", Value: strings.Join(locality.Spec.Ingestors, " ")},
		{Name: "KR_LOCALITY", ValueFrom: &v12.EnvVarSource{
			FieldRef: &v12.ObjectFieldSelector{
				FieldPath: "metadata.namespace",
			},
		}},
	}

	containers := []v12.Container{{
		Name:            jobName,
		Image:           keyRotatorImage,
		Env:             env,
		ImagePullPolicy: v12.PullAlways,
	}}

	objectMeta := metav1.ObjectMeta{
		Name:      jobName,
		Namespace: locality.Namespace,
	}

	return v1beta1.CronJob{
		ObjectMeta: objectMeta,
		Spec: v1beta1.CronJobSpec{
			Schedule:          locality.Spec.Schedule,
			ConcurrencyPolicy: v1beta1.ReplaceConcurrent,
			JobTemplate: v1beta1.JobTemplateSpec{
				Spec: v1.JobSpec{
					Template: v12.PodTemplateSpec{
						Spec: v12.PodSpec{
							Containers:                   containers,
							RestartPolicy:                v12.RestartPolicyOnFailure,
							ServiceAccountName:           serviceAccountName,
							AutomountServiceAccountToken: &trueValue,
						},
					},
					TTLSecondsAfterFinished: &jobTTL,
				},
			},
		},
	}
}

func (r *LocalityReconciler) validateCurrentJob(existingCronJob *v1beta1.CronJob, locality *priov1.Locality) (ctrl.Result, error) {
	expectedCronJob := r.createCronJobTemplate(locality)
	if equality.Semantic.DeepDerivative(expectedCronJob.Spec, existingCronJob.Spec) {
		return ctrl.Result{}, nil
	}
	r.Log.
		WithValues("Existing Job", existingCronJob.Spec).
		WithValues("New Job", expectedCronJob.Spec).
		Info("The CronJob definition has changed - we're going to update the existing job")

	existingCronJob.Spec = expectedCronJob.Spec

	err := r.Client.Update(r.ctx, existingCronJob)
	if err != nil {
		return ctrl.Result{Requeue: true}, err
	}

	return ctrl.Result{}, nil
}

func (r *LocalityReconciler) validate(locality *priov1.Locality) (ctrl.Result, error) {
	// TODO: Do we want any validation on CRDs?
	return ctrl.Result{}, nil
}

func (r *LocalityReconciler) SetupWithManager(mgr ctrl.Manager) error {
	if err := mgr.GetFieldIndexer().IndexField(&v1beta1.CronJob{}, jobNameField, func(rawObj runtime.Object) []string {
		job := rawObj.(*v1beta1.CronJob)

		return []string{job.Name}
	}); err != nil {
		return err
	}
	return ctrl.NewControllerManagedBy(mgr).
		For(&priov1.Locality{}).
		Owns(&v1beta1.CronJob{}).
		Complete(r)
}
