use anyhow::{Context, Result, bail, ensure};
use arrow_array::{
    Array, BinaryArray, BinaryViewArray, LargeBinaryArray, LargeStringArray, RecordBatch,
    StringArray, StringViewArray,
};
use parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};
use std::{fs::File, path::Path};

pub const MAX_CLONED_BATCH_BYTES: usize = 256 * 1024 * 1024;
const MAX_COLUMN_CHUNK_UNCOMPRESSED_BYTES: i64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ParquetColumns {
    pub content: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RawDocument {
    pub id: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct ParquetBatch {
    pub documents: Vec<RawDocument>,
    pub input_documents: u64,
    pub input_bytes: u64,
    pub rejected_too_large: u64,
}

pub fn read_parquet_batches(
    path: &Path,
    columns: &ParquetColumns,
    batch_rows: usize,
    maximum_document_bytes: usize,
    mut consume: impl FnMut(ParquetBatch) -> Result<()>,
) -> Result<()> {
    ensure!(batch_rows > 0, "Parquet batch size must be positive");
    ensure!(
        maximum_document_bytes > 0,
        "maximum document size must be positive"
    );
    let maximum_cloned_bytes = batch_rows
        .checked_mul(maximum_document_bytes)
        .context("Parquet batch clone budget overflow")?;
    ensure!(
        maximum_cloned_bytes <= MAX_CLONED_BATCH_BYTES,
        "Parquet batch could clone {maximum_cloned_bytes} bytes, above the hard {} byte budget",
        MAX_CLONED_BATCH_BYTES
    );
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    for (row_group_index, row_group) in builder.metadata().row_groups().iter().enumerate() {
        for (column_index, column) in row_group.columns().iter().enumerate() {
            let size = column.uncompressed_size();
            ensure!(
                (0..=MAX_COLUMN_CHUNK_UNCOMPRESSED_BYTES).contains(&size),
                "Parquet row group {row_group_index}, column {column_index} declares {size} uncompressed bytes; rewrite the shard with bounded pages/row groups"
            );
        }
    }
    let schema = builder.schema().clone();
    let content_index = schema
        .index_of(&columns.content)
        .with_context(|| format!("missing content column {:?}", columns.content))?;
    let id_index = columns
        .id
        .as_ref()
        .map(|name| {
            schema
                .index_of(name)
                .with_context(|| format!("missing id column {name:?}"))
        })
        .transpose()?;
    let mut roots = vec![content_index];
    if let Some(index) = id_index {
        roots.push(index);
    }
    roots.sort_unstable();
    roots.dedup();
    let projection = ProjectionMask::roots(builder.parquet_schema(), roots);
    let reader = builder
        .with_projection(projection)
        .with_batch_size(batch_rows)
        .build()?;

    let mut global_row = 0_u64;
    for batch in reader {
        let batch = batch?;
        let documents = extract_documents(
            &batch,
            columns,
            path,
            &mut global_row,
            maximum_document_bytes,
        )?;
        consume(documents)?;
    }
    Ok(())
}

fn extract_documents(
    batch: &RecordBatch,
    columns: &ParquetColumns,
    path: &Path,
    global_row: &mut u64,
    maximum_document_bytes: usize,
) -> Result<ParquetBatch> {
    let content = batch.column_by_name(&columns.content).with_context(|| {
        format!(
            "projected batch omitted content column {:?}",
            columns.content
        )
    })?;
    let id = columns
        .id
        .as_ref()
        .and_then(|name| batch.column_by_name(name));
    let mut result = ParquetBatch {
        documents: Vec::with_capacity(batch.num_rows()),
        ..ParquetBatch::default()
    };
    for row in 0..batch.num_rows() {
        let row_number = *global_row;
        *global_row += 1;
        let Some(source) = byte_value(content.as_ref(), row)? else {
            continue;
        };
        result.input_documents += 1;
        result.input_bytes = result
            .input_bytes
            .checked_add(source.len() as u64)
            .context("Parquet input byte count overflow")?;
        if source.len() > maximum_document_bytes {
            result.rejected_too_large += 1;
            continue;
        }
        let document_id = match id {
            Some(array) => byte_value(array.as_ref(), row)?
                .filter(|value| !value.is_empty() && value.len() <= 1024 * 1024)
                .and_then(|value| std::str::from_utf8(value).ok())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}:{row_number}", path.display())),
            None => format!("{}:{row_number}", path.display()),
        };
        result.documents.push(RawDocument {
            id: document_id,
            content: source.to_vec(),
        });
    }
    Ok(result)
}

fn byte_value(array: &dyn Array, row: usize) -> Result<Option<&[u8]>> {
    if array.is_null(row) {
        return Ok(None);
    }
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(Some(values.value(row).as_bytes()));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Ok(Some(values.value(row).as_bytes()));
    }
    if let Some(values) = array.as_any().downcast_ref::<StringViewArray>() {
        return Ok(Some(values.value(row).as_bytes()));
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryArray>() {
        return Ok(Some(values.value(row)));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        return Ok(Some(values.value(row)));
    }
    if let Some(values) = array.as_any().downcast_ref::<BinaryViewArray>() {
        return Ok(Some(values.value(row)));
    }
    bail!(
        "column type {:?} is unsupported; expected UTF-8 or binary content",
        array.data_type()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    #[test]
    fn reads_projected_content_and_ids_in_batches() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("content.parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("ignored", DataType::Int64, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("blob_id", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(arrow_array::Int64Array::from(vec![10_i64, 20])),
                Arc::new(StringArray::from(vec!["print(1)", "print(2)"])),
                Arc::new(StringArray::from(vec!["a", "b"])),
            ],
        )
        .unwrap();
        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let mut documents = Vec::new();
        read_parquet_batches(
            &path,
            &ParquetColumns {
                content: "content".into(),
                id: Some("blob_id".into()),
            },
            1,
            16 * 1024 * 1024,
            |batch| {
                documents.extend(batch.documents);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].id, "a");
        assert_eq!(documents[1].content, b"print(2)");
    }

    #[test]
    fn preserves_invalid_utf8_for_per_document_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("binary.parquet");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "content",
            DataType::Binary,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from_iter_values([
                &b"valid"[..],
                &[0xff, 0xfe],
            ]))],
        )
        .unwrap();
        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let mut documents = Vec::new();
        read_parquet_batches(
            &path,
            &ParquetColumns {
                content: "content".into(),
                id: None,
            },
            2,
            16,
            |batch| {
                documents.extend(batch.documents);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(documents.len(), 2);
        assert!(std::str::from_utf8(&documents[1].content).is_err());
    }

    #[test]
    fn rejects_a_clone_budget_above_the_hard_limit() {
        let result = read_parquet_batches(
            Path::new("not-opened.parquet"),
            &ParquetColumns {
                content: "content".into(),
                id: None,
            },
            17,
            16 * 1024 * 1024,
            |_| Ok(()),
        );
        assert!(result.is_err());
    }
}
