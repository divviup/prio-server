package main

import (
	"fmt"
	"reflect"
	"testing"
	"time"

	"github.com/letsencrypt/prio-server/workflow-manager/task"
	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
)

type mockEnqueuer struct {
	enqueuedTasks []task.Task
}

func (e *mockEnqueuer) Enqueue(task task.Task, completion func(error)) {
	e.enqueuedTasks = append(e.enqueuedTasks, task)
	completion(nil)
}

func (e *mockEnqueuer) Stop() {}

type mockBucket struct {
	aggregationIDs       []string
	batchFiles           []string
	intakeTaskMarkers    []string
	aggregateTaskMarkers []string
	writtenObjectKeys    []string
}

func (b *mockBucket) ListAggregationIDs() ([]string, error) {
	return b.aggregationIDs, nil
}

func (b *mockBucket) ListBatchFiles(aggregationID string, interval wftime.Interval) ([]string, error) {
	return b.batchFiles, nil
}

func (b *mockBucket) ListIntakeTaskMarkers(aggregationID string, interval wftime.Interval) ([]string, error) {
	return b.intakeTaskMarkers, nil
}

func (b *mockBucket) ListAggregateTaskMarkers(aggregationID string) ([]string, error) {
	return b.aggregateTaskMarkers, nil
}

func (b *mockBucket) WriteTaskMarker(marker string) error {
	b.writtenObjectKeys = append(b.writtenObjectKeys, fmt.Sprintf("task-markers/%s", marker))
	return nil
}

func TestScheduleIntakeTasks(t *testing.T) {
	batchTime, _ := time.Parse("2006/01/02/15/04", "2020/10/31/20/29")
	within24Hours, _ := time.Parse("2006/01/02/15/04", "2020/10/31/23/29")
	maxAge, _ := time.ParseDuration("24h")
	aggregationPeriod, _ := time.ParseDuration("8h")
	gracePeriod, _ := time.ParseDuration("4h")
	intakeMarker := "intake-kittens-seen-2020-10-31-20-29-b8a5579a-f984-460a-a42d-2813cbf57771"

	var testCases = []struct {
		name               string
		taskMarkerExists   bool
		expectedIntakeTask *task.IntakeBatch
		expectedTaskMarker string
	}{
		{
			name:             "current-batch-no-marker",
			taskMarkerExists: false,
			expectedIntakeTask: &task.IntakeBatch{
				AggregationID: "kittens-seen",
				BatchID:       "b8a5579a-f984-460a-a42d-2813cbf57771",
				Date:          wftime.Timestamp(batchTime),
			},
			expectedTaskMarker: intakeMarker,
		},
		{
			name:               "current-batch-has-marker",
			taskMarkerExists:   true,
			expectedIntakeTask: nil,
			expectedTaskMarker: "",
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			clock := wftime.ClockWithFixedNow(within24Hours)

			intakeBucket := mockBucket{
				aggregationIDs: []string{"kittens-seen"},
				batchFiles: []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.sig",
				},
			}

			ownValidationBucket := mockBucket{
				aggregationIDs: []string{"kittens-seen"},
			}

			if testCase.taskMarkerExists {
				ownValidationBucket.intakeTaskMarkers = []string{intakeMarker}
			}

			peerValidationBucket := mockBucket{
				aggregationIDs: []string{"kittens-seen"},
			}

			intakeTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			aggregateTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}

			err := scheduleTasks(scheduleTasksConfig{
				aggregationID:           "kittens-seen",
				isFirst:                 false,
				clock:                   clock,
				intakeBucket:            &intakeBucket,
				ownValidationBucket:     &ownValidationBucket,
				peerValidationBucket:    &peerValidationBucket,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
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
					if object == "task-markers/"+testCase.expectedTaskMarker {
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
	withinWindow, _ := time.Parse("2006/01/02/15/04", "2020/11/01/04/01")
	maxAge, _ := time.ParseDuration("24h")
	aggregationPeriod, _ := time.ParseDuration("8h")
	gracePeriod, _ := time.ParseDuration("4h")
	aggregationMarker := "aggregate-kittens-seen-2020-10-31-16-00-2020-11-01-00-00"
	expectedAggregationTask := &task.Aggregation{
		AggregationID:    "kittens-seen",
		AggregationStart: wftime.Timestamp(aggregationStart),
		AggregationEnd:   wftime.Timestamp(aggregationEnd),
		Batches: []task.Batch{
			task.Batch{
				ID:   "b8a5579a-f984-460a-a42d-2813cbf57771",
				Time: wftime.Timestamp(batchTime),
			},
		},
	}

	var testCases = []struct {
		name                    string
		hasOwnValidation        bool
		hasPeerValidation       bool
		taskMarkerExists        bool
		expectedAggregationTask *task.Aggregation
		expectedTaskMarker      string
	}{
		{
			name:                    "within-window-no-own-no-peer",
			hasOwnValidation:        false,
			hasPeerValidation:       false,
			taskMarkerExists:        false,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-no-own-has-peer",
			hasOwnValidation:        false,
			hasPeerValidation:       true,
			taskMarkerExists:        false,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-has-own-no-peer",
			hasOwnValidation:        true,
			hasPeerValidation:       false,
			taskMarkerExists:        false,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "within-window-no-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			taskMarkerExists:        false,
			expectedAggregationTask: expectedAggregationTask,
			expectedTaskMarker:      aggregationMarker,
		},
		{
			name:                    "within-window-has-marker",
			hasOwnValidation:        true,
			hasPeerValidation:       true,
			taskMarkerExists:        true,
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			clock := wftime.ClockWithFixedNow(withinWindow)

			intakeBucket := mockBucket{
				aggregationIDs: []string{"kittens-seen"},
				batchFiles: []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.sig",
				},
			}

			ownValidationBucket := mockBucket{
				aggregationIDs:    []string{"kittens-seen"},
				intakeTaskMarkers: []string{"intake-kittens-seen-2020-10-31-20-29-b8a5579a-f984-460a-a42d-2813cbf57771"},
			}

			if testCase.hasOwnValidation {
				ownValidationBucket.batchFiles = []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_1.sig",
				}
			}

			if testCase.taskMarkerExists {
				ownValidationBucket.aggregateTaskMarkers = []string{aggregationMarker}
			}

			peerValidationBucket := mockBucket{
				aggregationIDs: []string{"kittens-seen"},
			}

			if testCase.hasPeerValidation {
				peerValidationBucket.batchFiles = []string{
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.avro",
					"kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.sig",
				}
			}

			intakeTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			aggregateTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}

			err := scheduleTasks(scheduleTasksConfig{
				aggregationID:           "kittens-seen",
				isFirst:                 false,
				clock:                   clock,
				intakeBucket:            &intakeBucket,
				ownValidationBucket:     &ownValidationBucket,
				peerValidationBucket:    &peerValidationBucket,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
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
					if object == "task-markers/"+testCase.expectedTaskMarker {
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
