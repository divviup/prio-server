package main

import (
	"fmt"
	"path"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"

	"github.com/letsencrypt/prio-server/workflow-manager/task"
	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
)

var expectedUuid = uuid.New()

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
	var result []string
	for _, ts := range interval.TimestampPrefixes() {
		prefix := path.Join(aggregationID, ts.TruncatedTimestamp())
		for _, bf := range b.batchFiles {
			if strings.HasPrefix(bf, prefix) {
				result = append(result, bf)
			}
		}
	}
	return result, nil
}

func (b *mockBucket) ListIntakeTaskMarkers(aggregationID string, interval wftime.Interval) ([]string, error) {
	var result []string
	for _, ts := range interval.TimestampPrefixes() {
		prefix := fmt.Sprintf("intake-%s-%s", aggregationID, ts.TruncatedMarkerString())
		for _, itm := range b.intakeTaskMarkers {
			if strings.HasPrefix(itm, prefix) {
				result = append(result, itm)
			}
		}
	}
	return result, nil
}

func (b *mockBucket) ListAggregateTaskMarkers(aggregationID string) ([]string, error) {
	var result []string
	prefix := fmt.Sprintf("aggregate-%s-", aggregationID)
	for _, atm := range b.aggregateTaskMarkers {
		if strings.HasPrefix(atm, prefix) {
			result = append(result, atm)
		}
	}
	return result, nil
}

func (b *mockBucket) WriteTaskMarker(marker string) error {
	b.writtenObjectKeys = append(b.writtenObjectKeys, fmt.Sprintf("task-markers/%s", marker))
	return nil
}

func TestScheduleIntakeTasks(t *testing.T) {
	batchTime := mustParseTime(t, "2020/10/31/20/29")
	now := mustParseTime(t, "2020/10/31/23/29") // within 24 hours of batchTime
	maxAge := 24 * time.Hour
	aggregationPeriod := 8 * time.Hour
	gracePeriod := 4 * time.Hour
	intakeMarker := "intake-kittens-seen-2020-10-31-20-29-b8a5579a-f984-460a-a42d-2813cbf57771"

	for _, testCase := range []struct {
		name               string
		taskMarkerExists   bool
		expectedIntakeTask *task.IntakeBatch
		expectedTaskMarker string
	}{
		{
			name:             "current-batch-no-marker",
			taskMarkerExists: false,
			expectedIntakeTask: &task.IntakeBatch{
				TraceID:       expectedUuid,
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
	} {
		t.Run(testCase.name, func(t *testing.T) {
			clock := wftime.ClockWithFixedNow(now)

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

			if err := scheduleTasks(scheduleTasksConfig{
				aggregationID:           "kittens-seen",
				isFirst:                 false,
				clock:                   clock,
				intakeBucket:            &intakeBucket,
				ownValidationBucket:     &ownValidationBucket,
				peerValidationBucket:    &peerValidationBucket,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
				maxAge:                  maxAge,
				aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			}); err != nil {
				t.Errorf("Unexpected error: %v", err)
			}

			if testCase.expectedIntakeTask == nil {
				if len(intakeTaskEnqueuer.enqueuedTasks) != 0 {
					t.Errorf("Unexpected intake tasks scheduled: %v", intakeTaskEnqueuer.enqueuedTasks)
				}
			} else {
				foundExpectedTask := false
				for _, enqueuedTask := range intakeTaskEnqueuer.enqueuedTasks {
					if intakeTask, ok := enqueuedTask.(task.IntakeBatch); ok {
						// TraceID is a dynamic value assigned at runtime. Don't
						// use it to match
						intakeTask.TraceID = expectedUuid

						if reflect.DeepEqual(intakeTask, *testCase.expectedIntakeTask) {
							foundExpectedTask = true
							break
						}
					}
				}
				if !foundExpectedTask {
					t.Errorf("Did not find expected intake task %+v among %v", testCase.expectedIntakeTask, intakeTaskEnqueuer.enqueuedTasks)
				}
			}

			if len(aggregateTaskEnqueuer.enqueuedTasks) != 0 {
				t.Errorf("Unexpected aggregation tasks scheduled: %v", aggregateTaskEnqueuer.enqueuedTasks)
			}

			if testCase.expectedTaskMarker == "" {
				if len(ownValidationBucket.writtenObjectKeys) != 0 {
					t.Errorf("Unexpected task marker written: %v", ownValidationBucket.writtenObjectKeys)
				}
			} else {
				foundExpectedMarker := false
				wantedObject := path.Join("task-markers", testCase.expectedTaskMarker)
				for _, object := range ownValidationBucket.writtenObjectKeys {
					if object == wantedObject {
						foundExpectedMarker = true
						break
					}
				}
				if !foundExpectedMarker {
					t.Errorf("Did not find expected task marker among %v", ownValidationBucket.writtenObjectKeys)
				}
			}
		})
	}
}

func TestScheduleAggregationTasks(t *testing.T) {
	batchTime := mustParseTime(t, "2020/10/31/02/29")
	aggregationStart := mustParseTime(t, "2020/10/31/00/00")
	aggregationEnd := mustParseTime(t, "2020/10/31/08/00")
	aggregationMidpoint := aggregationStart.Add(aggregationEnd.Sub(aggregationStart) / 2)
	now := mustParseTime(t, "2020/11/01/04/01")
	maxAge := 24 * time.Hour
	aggregationPeriod := 8 * time.Hour
	gracePeriod := 20 * time.Hour
	aggregationMarker := "aggregate-kittens-seen-2020-10-31-00-00-2020-10-31-08-00"
	expectedAggregationTask := &task.Aggregation{
		TraceID:          expectedUuid,
		AggregationID:    "kittens-seen",
		AggregationStart: wftime.Timestamp(aggregationStart),
		AggregationEnd:   wftime.Timestamp(aggregationEnd),
		Batches: []task.Batch{{
			ID:   "b8a5579a-f984-460a-a42d-2813cbf57771",
			Time: wftime.Timestamp(batchTime),
		}},
	}

	for _, testCase := range []struct {
		name                    string
		hasIntakeBatch          bool
		hasPeerValidation       bool
		taskMarkerExists        bool
		aggregationInterval     wftime.AggregationIntervalFunc
		expectedAggregationTask *task.Aggregation
		expectedTaskMarker      string
	}{
		// Standard aggregation window tests.
		{
			name:                    "standard-within-window-no-intake-no-peer",
			hasIntakeBatch:          false,
			hasPeerValidation:       false,
			taskMarkerExists:        false,
			aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "standard-within-window-no-intake-has-peer",
			hasIntakeBatch:          false,
			hasPeerValidation:       true,
			taskMarkerExists:        false,
			aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "standard-within-window-has-intake-no-peer",
			hasIntakeBatch:          true,
			hasPeerValidation:       false,
			taskMarkerExists:        false,
			aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
		{
			name:                    "standard-within-window-no-marker",
			hasIntakeBatch:          true,
			hasPeerValidation:       true,
			taskMarkerExists:        false,
			aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			expectedAggregationTask: expectedAggregationTask,
			expectedTaskMarker:      aggregationMarker,
		},
		{
			name:                    "standard-within-window-has-marker",
			hasIntakeBatch:          true,
			hasPeerValidation:       true,
			taskMarkerExists:        true,
			aggregationInterval:     wftime.StandardAggregationWindow(aggregationPeriod, gracePeriod),
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},

		// Override aggregation window tests.
		{
			name:                    "override-within-window-no-marker",
			hasIntakeBatch:          true,
			hasPeerValidation:       true,
			taskMarkerExists:        false,
			aggregationInterval:     wftime.OverrideAggregationWindow(aggregationMidpoint, aggregationPeriod),
			expectedAggregationTask: expectedAggregationTask,
			expectedTaskMarker:      aggregationMarker,
		},
		{
			name:                    "override-within-window-has-marker",
			hasIntakeBatch:          true,
			hasPeerValidation:       true,
			taskMarkerExists:        true,
			aggregationInterval:     wftime.OverrideAggregationWindow(aggregationMidpoint, aggregationPeriod),
			expectedAggregationTask: nil,
			expectedTaskMarker:      "",
		},
	} {
		t.Run(testCase.name, func(t *testing.T) {
			clock := wftime.ClockWithFixedNow(now)

			intakeBucket := mockBucket{aggregationIDs: []string{"kittens-seen"}}
			if testCase.hasIntakeBatch {
				intakeBucket.batchFiles = []string{
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch",
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.avro",
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.batch.sig",
				}
			}

			ownValidationBucket := mockBucket{
				aggregationIDs:    []string{"kittens-seen"},
				intakeTaskMarkers: []string{"intake-kittens-seen-2020-10-31-02-29-b8a5579a-f984-460a-a42d-2813cbf57771"},
			}
			if testCase.taskMarkerExists {
				ownValidationBucket.aggregateTaskMarkers = []string{aggregationMarker}
			}

			peerValidationBucket := mockBucket{aggregationIDs: []string{"kittens-seen"}}
			if testCase.hasPeerValidation {
				peerValidationBucket.batchFiles = []string{
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0",
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.avro",
					"kittens-seen/2020/10/31/02/29/b8a5579a-f984-460a-a42d-2813cbf57771.validity_0.sig",
				}
			}

			intakeTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}
			aggregateTaskEnqueuer := mockEnqueuer{enqueuedTasks: []task.Task{}}

			if err := scheduleTasks(scheduleTasksConfig{
				aggregationID:           "kittens-seen",
				isFirst:                 false,
				clock:                   clock,
				intakeBucket:            &intakeBucket,
				ownValidationBucket:     &ownValidationBucket,
				peerValidationBucket:    &peerValidationBucket,
				intakeTaskEnqueuer:      &intakeTaskEnqueuer,
				aggregationTaskEnqueuer: &aggregateTaskEnqueuer,
				maxAge:                  maxAge,
				aggregationInterval:     testCase.aggregationInterval,
			}); err != nil {
				t.Errorf("Unexpected error: %v", err)
			}

			if len(intakeTaskEnqueuer.enqueuedTasks) != 0 {
				t.Errorf("Unexpected intake tasks scheduled: %v", intakeTaskEnqueuer.enqueuedTasks)
			}

			if testCase.expectedAggregationTask == nil {
				if len(aggregateTaskEnqueuer.enqueuedTasks) != 0 {
					t.Errorf("Unexpected aggregation tasks scheduled: %v", aggregateTaskEnqueuer.enqueuedTasks)
				}
			} else {
				foundExpectedTask := false
				for _, enqueuedTask := range aggregateTaskEnqueuer.enqueuedTasks {
					if aggregationTask, ok := enqueuedTask.(task.Aggregation); ok {
						// TraceID is a dynamic value assigned at runtime. Don't
						// use it to match
						aggregationTask.TraceID = expectedUuid

						if reflect.DeepEqual(aggregationTask, *testCase.expectedAggregationTask) {
							foundExpectedTask = true
							break
						}
					}
				}
				if !foundExpectedTask {
					t.Errorf("Did not find expected aggregate task among %v", aggregateTaskEnqueuer.enqueuedTasks)
				}
			}

			if testCase.expectedTaskMarker == "" {
				if len(ownValidationBucket.writtenObjectKeys) != 0 {
					t.Errorf("Unexpected task marker written: %v", ownValidationBucket.writtenObjectKeys)
				}
			} else {
				foundExpectedMarker := false
				wantedObject := path.Join("task-markers", testCase.expectedTaskMarker)
				for _, object := range ownValidationBucket.writtenObjectKeys {
					if object == wantedObject {
						foundExpectedMarker = true
						break
					}
				}
				if !foundExpectedMarker {
					t.Errorf("Did not find expected task marker among %v", ownValidationBucket.writtenObjectKeys)
				}
			}
		})
	}
}

func mustParseTime(t *testing.T, value string) time.Time {
	when, err := time.Parse("2006/01/02/15/04", value)
	if err != nil {
		t.Fatalf("Couldn't parse %q as time: %v", value, err)
	}
	return when
}
