package main

import (
	"fmt"
	"reflect"
	"testing"
	"time"

	"github.com/letsencrypt/prio-server/workflow-manager/batchpath"
	"github.com/letsencrypt/prio-server/workflow-manager/task"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"

	batchv1 "k8s.io/api/batch/v1"
	_ "k8s.io/api/core/v1"
)

func TestIntakeJobNameForBatchPath(t *testing.T) {
	var testCases = []struct {
		name     string
		input    string
		expected string
	}{
		{
			name:     "short-aggregation-name",
			input:    "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771",
			expected: "i-kittens-seen-b8a5579af984460a-2020-10-31-20-29",
		},
		{
			name:     "long-aggregation-name",
			input:    "a-very-long-aggregation-name-that-will-get-truncated/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771",
			expected: "i-a-very-long-aggregation-nam-b8a5579af984460a-2020-10-31-20-29",
		},
	}
	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			batchPath, err := batchpath.New(testCase.input)
			if err != nil {
				t.Fatalf("unexpected batch path parse failure: %s", err)
			}

			jobName := intakeJobNameForBatchPath(batchPath)
			if jobName != testCase.expected {
				t.Errorf("expected %q, encountered %q", testCase.expected, jobName)
			}

			if len(jobName) > 63 {
				t.Errorf("job name is too long")
			}
		})
	}
}

func TestAggregationJobNameFragment(t *testing.T) {
	input := "FooBar%012345678901234567890123456789"
	id := aggregationJobNameFragment(input, 30)
	expected := "foobar-01234567890123456789012"
	if id != expected {
		t.Errorf("expected id %q, got %q", expected, id)
	}
}

type mockEnqueuer struct {
	enqueuedTasks []task.Task
}

func (e *mockEnqueuer) Enqueue(task task.Task, completion func(error)) {
	e.enqueuedTasks = append(e.enqueuedTasks, task)
	completion(nil)
}

func (e *mockEnqueuer) Stop() {}

type mockBucket struct {
	writtenObjectKeys []string
}

func (b *mockBucket) WriteTaskMarker(marker string) error {
	b.writtenObjectKeys = append(b.writtenObjectKeys, fmt.Sprintf("task-markers/%s", marker))
	return nil
}

func TestScheduleIntakeTasks(t *testing.T) {
	batchTime, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/29")
	within24Hours, _ := time.Parse("2006/01/02/15/04", "2020/10/31/23/29")
	tooLate, _ := time.Parse("2006/01/02/15/04", "2020/11/02/20/29")
	maxAge, _ := time.ParseDuration("24h")
	aggregationPeriod, _ := time.ParseDuration("8h")
	gracePeriod, _ := time.ParseDuration("4h")
	intakeMarker := "task-markers/intake-kittens-seen-2020-10-31-20-29-b8a5579a-f984-460a-a42d-2813cbf57771"
	existingJob := "i-kittens-seen-b8a5579af984460a-2020-10-31-20-29"

	var testCases = []struct {
		name               string
		jobExists          bool
		taskMarkerExists   bool
		now                time.Time
		expectedIntakeTask *task.IntakeBatch
		expectedTaskMarker string
	}{
		{
			name:               "old-batch-no-job-no-marker",
			jobExists:          false,
			taskMarkerExists:   false,
			now:                tooLate,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
		{
			name:               "old-batch-no-job-has-marker",
			jobExists:          false,
			taskMarkerExists:   true,
			now:                tooLate,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
		{
			name:               "old-batch-has-job-no-marker",
			jobExists:          true,
			taskMarkerExists:   false,
			now:                tooLate,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
		{
			name:               "old-batch-has-job-has-marker",
			jobExists:          true,
			taskMarkerExists:   false,
			now:                tooLate,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
		{
			name:             "current-batch-no-job-no-marker",
			jobExists:        false,
			taskMarkerExists: false,
			now:              within24Hours,
			expectedIntakeTask: &task.IntakeBatch{
				AggregationID: "kittens-seen",
				BatchID:       "b8a5579a-f984-460a-a42d-2813cbf57771",
				Date:          task.Timestamp(batchTime),
			},
			expectedTaskMarker: intakeMarker,
		},
		{
			name:               "current-batch-no-job-has-marker",
			jobExists:          false,
			taskMarkerExists:   true,
			now:                within24Hours,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
		{
			name:               "current-batch-has-job-no-marker",
			jobExists:          true,
			taskMarkerExists:   false,
			now:                within24Hours,
			expectedIntakeTask: nil,
			expectedTaskMarker: intakeMarker,
		},
		{
			name:               "current-batch-has-job-has-marker",
			jobExists:          true,
			taskMarkerExists:   true,
			now:                within24Hours,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			clock := utils.ClockWithFixedNow(testCase.now)

			ownValidationFiles := []string{}
			if testCase.taskMarkerExists {
				ownValidationFiles = append(ownValidationFiles, intakeMarker)
			}

			peerValidationFiles := []string{}

			existingJobs := map[string]batchv1.Job{}
			if testCase.jobExists {
				existingJobs[existingJob] = batchv1.Job{}
			}

			intakeTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			aggregateTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			ownValidationBucket := mockBucket{writtenObjectKeys: []string{}}

			err := scheduleTasks(scheduleTasksConfig{
				isFirst: false,
				clock:   clock,
				intakeFiles: []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.sig",
				},
				ownValidationFiles:      ownValidationFiles,
				peerValidationFiles:     peerValidationFiles,
				existingJobs:            existingJobs,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
				ownValidationBucket:     &ownValidationBucket,
				maxAge:                  maxAge,
				aggregationPeriod:       aggregationPeriod,
				gracePeriod:             gracePeriod,
			})
			if err != nil {
				t.Errorf("unexpected error %q", err)
			}

			if testCase.expectedIntakeTask == nil {
				if len(intakeTaskEnqueuer.enqueuedTasks) != 0 {
					t.Errorf("unexpected intake tasks scheduled: %q", intakeTaskEnqueuer.enqueuedTasks)
				}
			} else {
				foundExpectedTask := false
				for _, task := range intakeTaskEnqueuer.enqueuedTasks {
					if reflect.DeepEqual(task, *testCase.expectedIntakeTask) {
						foundExpectedTask = true
						break
					}
				}
				if !foundExpectedTask {
					t.Errorf("did not find expected intake task %+v among %q", testCase.expectedIntakeTask, intakeTaskEnqueuer.enqueuedTasks)
				}
			}

			if len(aggregateTaskEnqueuer.enqueuedTasks) != 0 {
				t.Errorf("unexpected aggregation tasks scheduled: %q", aggregateTaskEnqueuer.enqueuedTasks)
			}

			if testCase.expectedTaskMarker == "" {
				if len(ownValidationBucket.writtenObjectKeys) != 0 {
					t.Errorf("unexpected task marker written: %q", ownValidationBucket.writtenObjectKeys)
				}
			} else {
				foundExpectedMarker := false
				for _, object := range ownValidationBucket.writtenObjectKeys {
					if object == testCase.expectedTaskMarker {
						foundExpectedMarker = true
						break
					}
				}
				if !foundExpectedMarker {
					t.Errorf("did not find expected task marker among %q", ownValidationBucket.writtenObjectKeys)
				}
			}
		})
	}
}

func TestScheduleAggregationTasks(t *testing.T) {
	batchTime, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/29")
	aggregationStart, _ := time.Parse("2006/01/02/15/04", "2020/10/31/16/00")
	aggregationEnd, _ := time.Parse("2006/01/02/15/04", "2020/11/01/00/00")
	tooSoon, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/29")
	tooLate, _ := time.Parse("2006/01/02/15/04", "2020/11/02/20/29")
	withinWindow, _ := time.Parse("2006/01/02/15/04", "2020/11/01/04/01")
	maxAge, _ := time.ParseDuration("24h")
	aggregationPeriod, _ := time.ParseDuration("8h")
	gracePeriod, _ := time.ParseDuration("4h")
	aggregationMarker := "task-markers/aggregate-kittens-seen-2020-10-31-16-00-2020-11-01-00-00"
	existingJob := "a-kittens-seen-2020-10-31-16-00"
	expectedAggregationTask := &task.Aggregation{
		AggregationID:    "kittens-seen",
		AggregationStart: task.Timestamp(aggregationStart),
		AggregationEnd:   task.Timestamp(aggregationEnd),
		Batches: []task.Batch{
			task.Batch{
				ID:   "b8a5579a-f984-460a-a42d-2813cbf57771",
				Time: task.Timestamp(batchTime),
			},
		},
	}

	var testCases = []struct {
		name                    string
		hasOwnValidation        bool
		hasPeerValidation       bool
		jobExists               bool
		taskMarkerExists        bool
		now                     time.Time
		expectedAggregationTask *task.Aggregation
		expectedTaskMarker      string
	}{
		{
			name:                    "too-soon-no-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     tooSoon,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-soon-no-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        true,
			now:                     tooSoon,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-soon-has-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        false,
			now:                     tooSoon,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-soon-has-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        true,
			now:                     tooSoon,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-late-no-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     tooLate,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-late-no-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        true,
			now:                     tooLate,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-late-has-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        false,
			now:                     tooLate,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "too-late-has-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        true,
			now:                     tooLate,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-no-own-no-peer",
			hasOwnValidation:        false,
			hasPeerValidation:       false,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-no-own-has-peer",
			hasOwnValidation:        false,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-has-own-no-peer",
			hasOwnValidation:        true,
			hasPeerValidation:       false,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-no-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        false,
			now:                     withinWindow,
			expectedAggregationTask: expectedAggregationTask,
			expectedTaskMarker:      aggregationMarker,
		},
		{
			name:                    "within-window-no-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               false,
			taskMarkerExists:        true,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-has-job-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        false,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      aggregationMarker,
		},
		{
			name:                    "within-window-has-job-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			jobExists:               true,
			taskMarkerExists:        true,
			now:                     withinWindow,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			clock := utils.ClockWithFixedNow(testCase.now)

			ownValidationFiles := []string{
				"task-markers/intake-kittens-seen-2020-10-31-20-29-b8a5579a-f984-460a-a42d-2813cbf57771",
			}
			if testCase.hasOwnValidation {
				ownValidationFiles = append(ownValidationFiles, []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1.sig",
				}...)
			}

			if testCase.taskMarkerExists {
				ownValidationFiles = append(ownValidationFiles, aggregationMarker)
			}

			peerValidationFiles := []string{}
			if testCase.hasPeerValidation {
				peerValidationFiles = []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.sig",
				}
			}

			existingJobs := map[string]batchv1.Job{}
			if testCase.jobExists {
				existingJobs[existingJob] = batchv1.Job{}
			}

			intakeTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			aggregateTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			ownValidationBucket := mockBucket{writtenObjectKeys: []string{}}

			err := scheduleTasks(scheduleTasksConfig{
				isFirst: false,
				clock:   clock,
				intakeFiles: []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.sig",
				},
				ownValidationFiles:      ownValidationFiles,
				peerValidationFiles:     peerValidationFiles,
				existingJobs:            existingJobs,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
				ownValidationBucket:     &ownValidationBucket,
				maxAge:                  maxAge,
				aggregationPeriod:       aggregationPeriod,
				gracePeriod:             gracePeriod,
			})
			if err != nil {
				t.Errorf("unexpected error: %q", err)
			}

			if len(intakeTaskEnqueuer.enqueuedTasks) != 0 {
				t.Errorf("unexpected intake tasks scheduled: %q", intakeTaskEnqueuer.enqueuedTasks)
			}

			if testCase.expectedAggregationTask == nil {
				if len(aggregateTaskEnqueuer.enqueuedTasks) != 0 {
					t.Errorf("unexpected aggregation tasks scheduled: %q", aggregateTaskEnqueuer.enqueuedTasks)
				}
			} else {
				foundExpectedTask := false
				for _, task := range aggregateTaskEnqueuer.enqueuedTasks {
					if reflect.DeepEqual(task, *testCase.expectedAggregationTask) {
						foundExpectedTask = true
						break
					}
				}
				if !foundExpectedTask {
					t.Errorf("did not find expected aggregate task among %q", aggregateTaskEnqueuer.enqueuedTasks)
				}
			}

			if testCase.expectedTaskMarker == "" {
				if len(ownValidationBucket.writtenObjectKeys) != 0 {
					t.Errorf("unexpected task marker written: %q", ownValidationBucket.writtenObjectKeys)
				}
			} else {
				foundExpectedMarker := false
				for _, object := range ownValidationBucket.writtenObjectKeys {
					if object == testCase.expectedTaskMarker {
						foundExpectedMarker = true
						break
					}
				}
				if !foundExpectedMarker {
					t.Errorf("did not find expected task marker among %q", ownValidationBucket.writtenObjectKeys)
				}
			}
		})
	}
}
