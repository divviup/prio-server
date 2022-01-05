package batchpath

import (
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/rs/zerolog/log"

	wftime "github.com/letsencrypt/prio-server/workflow-manager/time"
	"github.com/letsencrypt/prio-server/workflow-manager/utils"
)

// BatchPath represents a relative path to a batch
type BatchPath struct {
	AggregationID  string
	dateComponents []string
	ID             string
	Time           time.Time

	headerObjectExists    bool
	packetObjectExists    bool
	signatureObjectExists bool
}

// List is a type alias for a slice of BatchPath pointers
type List []*BatchPath

// NewList creates a List from a slice of strings
func NewList(batchNames []string) (List, error) {
	list := List{}
	for _, batchName := range batchNames {
		batchPath, err := New(batchName)
		if err != nil {
			return nil, err
		}
		list = append(list, batchPath)
	}

	return list, nil
}

// Len returns the size of the slice representing the BatchPaths
func (bpl List) Len() int {
	return len(bpl)
}

// Returns if the ith item in List occurs before the jth item
func (bpl List) Less(i, j int) bool {
	return bpl[i].Time.Before(bpl[j].Time)
}

// Swap swaps the ith element in List with the jth element
func (bpl List) Swap(i, j int) {
	bpl[i], bpl[j] = bpl[j], bpl[i]
}

// WithinInterval returns the subset of the batches in the receiver that are
// within the given Interval.
func (bpl List) WithinInterval(interval wftime.Interval) []string {
	output := []string{}
	for _, bp := range bpl {
		if interval.Includes(bp.Time) {
			output = append(output, bp.path())
		}
	}

	return output
}

// New creates a new BatchPath from a batchName
func New(batchName string) (*BatchPath, error) {
	// batchName is like "kittens-seen/2020/10/31/20/29/b8a5579a-f984-460a-a42d-2813cbf57771"
	pathComponents := strings.Split(batchName, "/")
	if len(pathComponents) < 6 {
		return nil, fmt.Errorf("malformed batch name: %q", batchName)
	}
	batchID := pathComponents[len(pathComponents)-1]
	aggregationID := pathComponents[0]
	batchDate := pathComponents[1 : len(pathComponents)-1]

	if len(batchDate) != 5 {
		return nil, fmt.Errorf("malformed date in %q. Expected 5 date components, got %d", batchName, len(batchDate))
	}

	var dateComponents []int
	for _, c := range batchDate {
		parsed, err := strconv.ParseInt(c, 10, 64)
		switch {
		case err != nil:
			return nil, fmt.Errorf("parsing date component %q in %q: %w", c, batchName, err)
		case parsed > math.MaxInt:
			return nil, fmt.Errorf("parsing date component %q in %q: parsed value (%d) larger than maximum allowed value (%d)", c, batchName, parsed, math.MaxInt)
		case parsed < 0:
			return nil, fmt.Errorf("parsing date component %q in %q: parsed value (%d) smaller than minimum allowed value (%d)", c, batchName, parsed, 0)
		}
		dateComponents = append(dateComponents, int(parsed))
	}
	batchTime := time.Date(dateComponents[0], time.Month(dateComponents[1]),
		dateComponents[2], dateComponents[3], dateComponents[4], 0, 0, time.UTC)

	return &BatchPath{
		AggregationID:  aggregationID,
		dateComponents: batchDate,
		ID:             batchID,
		Time:           batchTime,
	}, nil
}

func (b *BatchPath) String() string {
	return fmt.Sprintf("{%s %s %s files:%d%d%d}",
		b.AggregationID, b.dateComponents, b.ID,
		utils.Index(!b.headerObjectExists),
		utils.Index(!b.packetObjectExists),
		utils.Index(!b.signatureObjectExists))
}

func (b *BatchPath) path() string {
	return strings.Join([]string{b.AggregationID, b.DateString(), b.ID}, "/")
}

// DateString returns the string date representation of BatchPath
func (b *BatchPath) DateString() string {
	return strings.Join(b.dateComponents, "/")
}

type ReadyBatchesResult struct {
	Batches              List
	IncompleteBatchCount int
}

// ReadyBatches scans the provided list of files looking for batches made up of
// a header, packet file and a signature, corresponding to the given infix. On
// success, returns the list of discovered batches and a count of batches
// ignored because they were incomplete. Returns an error on failure.
func ReadyBatches(files []string, infix string, acceptSignatureOnly bool) (*ReadyBatchesResult, error) {
	batches := make(map[string]*BatchPath)
	for _, name := range files {
		// Ignore task marker objects
		if strings.HasPrefix(name, "task-markers/") {
			continue
		}
		basename := basename(name, infix)
		b := batches[basename]
		var err error
		if b == nil {
			b, err = New(basename)
			if err != nil {
				return nil, err
			}
			batches[basename] = b
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s", infix)) {
			b.headerObjectExists = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.avro", infix)) {
			b.packetObjectExists = true
		}
		if strings.HasSuffix(name, fmt.Sprintf(".%s.sig", infix)) {
			b.signatureObjectExists = true
		}
	}

	var output []*BatchPath
	incompleteBatchCount := 0
	for _, v := range batches {
		// A validation or ingestion batch is not ready unless all three files
		// are present. This isn't true for sum parts, but workflow-manager
		// doesn't deal with those yet.
		if v.signatureObjectExists && (acceptSignatureOnly || (v.headerObjectExists && v.packetObjectExists)) {
			output = append(output, v)
		} else {
			log.Info().Msgf("ignoring incomplete batch %s", v)
			incompleteBatchCount++
		}
	}
	sort.Sort(List(output))

	return &ReadyBatchesResult{Batches: output, IncompleteBatchCount: incompleteBatchCount}, nil
}

// basename returns s, with any type suffixes stripped off. The type suffixes are determined by
// `infix`, which is one of "batch", "validity_0", or "validity_1".
func basename(s string, infix string) string {
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.avro", infix))
	s = strings.TrimSuffix(s, fmt.Sprintf(".%s.sig", infix))
	return s
}
