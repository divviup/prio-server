package controllers

import (
	"context"
	"fmt"
	v1 "github.com/abetterinternet/prio-server/deploy-operator/api/v1"
	. "github.com/onsi/ginkgo"
	. "github.com/onsi/gomega"
	v1batch "k8s.io/api/batch/v1"
	"k8s.io/api/batch/v1beta1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"time"
)

func getDataShareProcessors() []string {
	return []string{
		"google",
		"apple",
	}
}

var _ = Describe("Locality controller", func() {

	// Define utility constants for object names and testing timeouts/durations and intervals.
	const (
		LocalityName           = "narnia"
		LocalityNamespace      = "default"
		EnvironmentName        = "narnia-environment"
		Schedule               = "1 * * * *"
		ManifestBucketLocation = "Some Location"

		// Values for the polling
		timeout  = time.Second * 10
		interval = time.Millisecond * 200
	)

	getLocality := func() *v1.Locality {
		return &v1.Locality{
			TypeMeta: metav1.TypeMeta{
				APIVersion: "prio.isrg-prio.org/v1",
				Kind:       "Locality",
			},
			ObjectMeta: metav1.ObjectMeta{
				Name:      LocalityName,
				Namespace: LocalityNamespace,
			},

			Spec: v1.LocalitySpec{
				EnvironmentName:        EnvironmentName,
				ManifestBucketLocation: ManifestBucketLocation,
				DataShareProcessors:    getDataShareProcessors(),
				Schedule:               Schedule,
			},
		}
	}

	Context("When pushing and updating a new locality", func() {
		It("Should validate lifecycle of CRDs", func() {
			By("Creating a Locality")
			ctx := context.Background()

			locality := getLocality()
			Expect(k8sClient.Create(ctx, locality)).Should(Succeed())

			createdLocality := &v1.Locality{}

			localityLookupKey, err := client.ObjectKeyFromObject(locality)
			Expect(err).ToNot(HaveOccurred())

			Eventually(func() bool {
				err = k8sClient.Get(ctx, localityLookupKey, createdLocality)

				if err != nil {
					return false
				}
				return true
			}, timeout, interval).Should(BeTrue())

			// Validate that it created it properly
			Expect(createdLocality.Spec).Should(Equal(locality.Spec))

			By("Checking to see if a CronJob has been made")

			createdCronJob := &v1beta1.CronJob{}
			cronJobLookupKey := client.ObjectKey{
				Name:      jobName,
				Namespace: LocalityNamespace,
			}
			Eventually(func() bool {
				err = k8sClient.Get(ctx, cronJobLookupKey, createdCronJob)

				if err != nil {
					return false
				}
				return true
			}, timeout, interval).Should(BeTrue())

			Expect(createdCronJob.Spec.Schedule).Should(Equal(Schedule))

			By("Checking to see if the Job has been made too")

			createdJob := &v1batch.Job{}
			jobLookupKey := client.ObjectKey{
				Name:      fmt.Sprintf("init-%s", jobName),
				Namespace: LocalityNamespace,
			}
			Eventually(func() bool {
				err = k8sClient.Get(ctx, jobLookupKey, createdJob)

				if err != nil {
					return false
				}
				return true
			}, timeout, interval).Should(BeTrue())

			Expect(createdJob.Spec.Template.Spec.Containers[0].Image).Should(Equal(keyRotatorImage))

			By("Updating the CRD")
			createdLocality.Spec.ManifestBucketLocation = "Some other manifest bucket location"
			Expect(k8sClient.Update(ctx, createdLocality)).Should(Succeed())

			By("Then verifying the CronJob has been updated as well")

			Eventually(func() bool {
				err := k8sClient.Get(ctx, cronJobLookupKey, createdCronJob)

				if err != nil {
					return false
				}
				for _, envVar := range createdCronJob.Spec.JobTemplate.Spec.Template.Spec.Containers[0].Env {
					if envVar.Value == createdLocality.Spec.ManifestBucketLocation {
						return true
					}
				}
				return false
			}, timeout, interval).Should(BeTrue())

		})

	})

})
