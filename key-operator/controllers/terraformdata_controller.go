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
	key_generator "github.com/abetterinternet/key-operator/key"
	v1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/go-logr/logr"
	"k8s.io/apimachinery/pkg/runtime"
	ctrl "sigs.k8s.io/controller-runtime"
	"sigs.k8s.io/controller-runtime/pkg/client"

	terraformv1 "github.com/abetterinternet/key-operator/api/v1"
)

// TerraformDataReconciler reconciles a TerraformData object
type TerraformDataReconciler struct {
	client.Client
	Log    logr.Logger
	Scheme *runtime.Scheme
}

// +kubebuilder:rbac:groups=terraform.isrg-prio.com,resources=terraformdata,verbs=get;list;watch;create;update;patch;delete
// +kubebuilder:rbac:groups=terraform.isrg-prio.com,resources=terraformdata/status,verbs=get;update;patch

func (r *TerraformDataReconciler) Reconcile(req ctrl.Request) (ctrl.Result, error) {
	ctx := context.Background()
	_ = r.Log.WithValues("terraformdata", req.NamespacedName)

	// your logic here
	data := &terraformv1.TerraformData{}
	err := r.Get(ctx, req.NamespacedName, data)
	if err != nil {
		if errors.IsNotFound(err) {
			return ctrl.Result{}, nil
		}
		r.Log.Error(err, "Failed to get TerraformData")
		return ctrl.Result{}, err
	}

	if !data.Status.KeyCreated {
		key, err := key_generator.GenerateKey()
		if err != nil {
			r.Log.Error(err, "Unable to create private key")
		}

		err = r.storePrivateKey(key, data.Spec.HealthAuthorityName, req.Namespace, data)
		if err != nil {
			r.Log.Error(err, "Unable to store private key")
		}

	}

	//r.Update(ctx, data)

	return ctrl.Result{}, nil
}

func (r *TerraformDataReconciler) storePrivateKey(key, name, namespace string, terraformData *terraformv1.TerraformData) error {
	immutable := true
	secret := &v1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: namespace,
			Labels: map[string]string{
				"terraform.isrg-prio.com/packet_encryption_key": "",
			},
		},
		Immutable: &immutable,
		StringData: map[string]string{
			"signing_key": key,
		},
	}

	// Can only error if the secret already has an owner
	_ = ctrl.SetControllerReference(terraformData, secret, r.Scheme)

	return r.Create(context.Background(), secret)
}

func (r *TerraformDataReconciler) SetupWithManager(mgr ctrl.Manager) error {
	return ctrl.NewControllerManagedBy(mgr).
		For(&terraformv1.TerraformData{}).
		Complete(r)
}
